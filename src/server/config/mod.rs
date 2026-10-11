//! Configuration — the two files, where they live, and the values they carry. One `Config` is
//! loaded at startup; only this layer opens files. Details in this folder's `README.md`.

pub mod paths;
pub mod prompts;
pub mod read;

use anyhow::{Context as _, anyhow};

use crate::server::config::paths::ConfigPaths;

// The public surface, re-exported so a caller reaches for one name (`config::OutputLimits`).
pub use read::config::{
    AgentConfig, ApiConfig, AppConfig, BrowserConfig, ColorSpec, CompactConfig, ConfigFile, Fence,
    FxConfig, LoopConfig, OutputLimits, PlaceholderConfig, PlatformConfig, RequestConfig,
    ServerConfig, StampConfig, StorageConfig, StreamMode, Theme, TodoFold, ToolsConfig, TuiConfig,
    UsageAlign, UsageConfig, WebConfig,
};
pub use read::model::{
    Cost, Currency, ModelEntry, ModelsConfig, Provider, ResolvedModel, default_api, normalize_api,
};

/// Everything this layer hands out: the two files' contents, the roots this load used, and the
/// `config.yaml` path a default-model write edits.
#[derive(Debug, Clone)]
pub struct Config {
    pub models: ModelsConfig,
    pub app: AppConfig,
    pub paths: ConfigPaths,
    app_file: std::path::PathBuf,
}

impl Config {
    /// Persist `id` as the default model in the `config.yaml` this config came from, and return
    /// its path. The id has to resolve first: a stored id no model answers to breaks the next start.
    pub fn set_default_model(&self, id: &str) -> anyhow::Result<std::path::PathBuf> {
        self.model_by_id(id)?;
        write_key(
            &self.app_file,
            &["config", "server", "api", "defaultModel"],
            id,
        )?;
        Ok(self.app_file.clone())
    }

    /// Load both files from the standard roots (see [`ConfigPaths::from_env`]).
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(&ConfigPaths::from_env())
    }

    /// Load both files from the roots in `paths`.
    pub fn load_from(paths: &ConfigPaths) -> anyhow::Result<Self> {
        // The compatibility rule table takes its final shape here: a broken table fails at startup.
        crate::server::agent::request::assembly::compat::rules::install()?;

        const TEMPLATE_URL: &str =
            "https://raw.githubusercontent.com/yanyjb1/MyPi/main/models_example.yml";
        let models_path = match paths.models_files().first() {
            Some((_, p)) => p.clone(),
            None => {
                let looked: Vec<String> = paths
                    .roots()
                    .iter()
                    .map(|(o, root)| {
                        format!("{}: {}", o.label(), root.join(paths::MODELS_FILE).display())
                    })
                    .collect();
                anyhow::bail!(
                    "no models.yml in any config root — create it yourself (the program never writes this file):\n  1. get the template: {TEMPLATE_URL}\n  2. put it at {}, fill in your providers and keys\n  looked in:\n    {}",
                    paths
                        .roots()
                        .first()
                        .map(|(_, root)| root.join(paths::MODELS_FILE).display().to_string())
                        .unwrap_or_default(),
                    looked.join("\n    ")
                );
            }
        };
        let models_text = std::fs::read_to_string(&models_path)
            .with_context(|| format!("failed to read {}", models_path.display()))?;
        let models: ModelsConfig = serde_yaml::from_str(&models_text)
            .with_context(|| format!("invalid models.yml format: {}", models_path.display()))?;

        // Every provider's `api` must name a protocol this binary implements: a typo fails at
        // startup, with a message the user can act on.
        for (name, provider) in &models.providers {
            if normalize_api(&provider.api).is_none() {
                let known = "openai-completions";
                anyhow::bail!(
                    "provider `{name}` declares api `{}`, which this program does not know. Implemented: {known} \
                     (the chat/completions wire protocol); responses and anthropic-messages are not supported.",
                    provider.api
                );
            }
        }

        let (app, app_file): (AppConfig, std::path::PathBuf) =
            match paths.config_files().first().map(|(_, p)| p.clone()) {
                Some(path) => {
                    let text = std::fs::read_to_string(&path)
                        .with_context(|| format!("failed to read {}", path.display()))?;
                    // A file that does not parse is not second-guessed: the user gets the
                    // choice of fixing it or deleting it, and nothing else.
                    let file: ConfigFile = serde_yaml::from_str(&text).map_err(|e| {
                        anyhow!(
                            "{} 读不了：{e}\n\
                             要么把它修好，要么把它删了（删了会照默认重建一份）。二选一，我等你。",
                            path.display()
                        )
                    })?;
                    (file.config, path)
                }
                None => {
                    // Bootstrap: the alphabetically-first display name, persisted (config.yaml is
                    // program-owned, unlike models.yml which is never written).
                    let first = Self::alphabetical_default(&models).ok_or_else(|| {
                        anyhow!("models.yml declares no models — add at least one `- id: ...` entry")
                    })?;
                    let root = paths.global_root().ok_or_else(|| {
                        anyhow!(
                            "no {} under {}, and no global root to write one to — \
                             MYPI_CONFIG_DIR 指到哪个目录就只认哪个目录，这里不做兜底",
                            paths::CONFIG_FILE,
                            paths
                                .roots()
                                .iter()
                                .map(|(_, r)| r.display().to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?;
                    let mut app = AppConfig::default();
                    app.server.api.default_model = Some(first);
                    let path = root.join(paths::CONFIG_FILE);
                    std::fs::create_dir_all(root)?;
                    let yaml = serde_yaml::to_string(&ConfigFile {
                        config: app.clone(),
                    })?;
                    std::fs::write(&path, &yaml)
                        .with_context(|| format!("failed to write {}", path.display()))?;
                    // A machine that has never run this: say what was written,
                    // so the defaults are visible rather than merely in effect.
                    println!("mypi: 没有配置文件，照默认生成了一份：{}", path.display());
                    println!("{yaml}");
                    println!("mypi: 想改就改这一份；不改，就照它走。");
                    (app, path)
                }
            };

        let cfg = Config {
            models,
            app,
            paths: paths.clone(),
            app_file,
        };
        Self::finish_load(cfg)
    }

    /// Validate the loaded config and make a stale `default` survivable.
    fn finish_load(mut cfg: Config) -> anyhow::Result<Config> {
        cfg.validate()?;
        if let Some(stale) = cfg.app.server.api.default_model.clone()
            && let Err(e) = cfg.model_by_id(&stale)
        {
            let fallback = Self::alphabetical_default(&cfg.models);
            eprintln!(
                "mypi: default model {stale} no longer resolves ({e:#}); using {} this time — re-pick it with /model",
                fallback.as_deref().unwrap_or("(none)")
            );
            cfg.app.server.api.default_model = fallback;
        }
        Ok(cfg)
    }

    /// The alphabetically-first model by display name, addressed as `<provider>:<id>`: the
    /// deterministic choice when config.yaml has no default yet.
    fn alphabetical_default(models: &ModelsConfig) -> Option<String> {
        let mut all: Vec<(String, String, String)> = models
            .providers
            .iter()
            .flat_map(|(p, prov)| {
                prov.models
                    .iter()
                    .map(move |m| (m.display_name().to_lowercase(), p.clone(), m.id.clone()))
            })
            .collect();
        all.sort();
        all.first().map(|(_, p, id)| format!("{p}:{id}"))
    }

    /// Fail fast: model ids are **provider-scoped** (the same id may
    /// appear under two providers), so uniqueness is checked per provider.
    fn validate(&self) -> anyhow::Result<()> {
        let mut any = false;
        for (pname, p) in &self.models.providers {
            let mut seen = std::collections::HashSet::new();
            for m in &p.models {
                any = true;
                if !seen.insert(&m.id) {
                    return Err(anyhow!("provider {pname}: duplicate model id `{}`", m.id));
                }
            }
        }
        if !any {
            return Err(anyhow!(
                "models.yml declares no models — add at least one `- id: ...` entry"
            ));
        }
        // The batch threshold must be positive: zero would make "land a batch every 0 tool calls"
        // a trap one keystroke away.
        if self.app.server.storage.batch_tools == 0 {
            return Err(anyhow!(
                "config.yaml → server.storage.batchTools 必须是正整数（0 没有意义）"
            ));
        }
        // Only the cutting rewind is built: a branch would need the block-sharing path, which
        // does not exist.
        if !self.app.server.prune_on_rewind {
            panic!("config.yaml → server.pruneOnRewind 只能是 true（开分支那条路还没做）");
        }
        // Structural only: a persisted default that no longer resolves is
        // handled by `finish_load` (warn + fall back), never here.
        Ok(())
    }

    /// Every declared model with its owning provider name, in config
    /// order (providers are a map, so sort names for a stable listing).
    pub fn models(&self) -> impl Iterator<Item = (&str, &ModelEntry)> {
        let mut names: Vec<&String> = self.models.providers.keys().collect();
        names.sort();
        names.into_iter().flat_map(move |n| {
            self.models.providers[n.as_str()]
                .models
                .iter()
                .map(move |m| (n.as_str(), m))
        })
    }

    /// The model every request uses unless the session says otherwise; falls back to
    /// `config.server.api.defaultModel`.
    pub fn default_model(&self) -> anyhow::Result<ResolvedModel> {
        let id = self
            .app
            .server
            .api
            .default_model
            .as_deref()
            .ok_or_else(|| anyhow!("no default model set — run /model <provider>:<id>"))?;
        self.model_by_id(id)
    }

    /// Find a model by its globally-addressed id `<provider>:<id>` — the form
    /// `default`, `/model`, `/switch` and completion all speak.
    pub fn model_by_id(&self, id: &str) -> anyhow::Result<ResolvedModel> {
        let (pname, mid) = id
            .split_once(':')
            .ok_or_else(|| anyhow!("model id must be `<provider>:<id>`, got `{id}`"))?;
        let p = self
            .models
            .providers
            .get(pname)
            .ok_or_else(|| anyhow!("unknown provider: {pname}"))?;
        let m = p
            .models
            .iter()
            .find(|m| m.id == mid)
            .ok_or_else(|| anyhow!("provider {pname} has no model `{mid}`"))?;
        Ok(ResolvedModel {
            provider_name: pname.to_string(),
            entry: m.clone(),
        })
    }

    /// Resolve `${ENV_VAR}` in an `api_key`, so the file never has to hold the
    /// secret itself.
    pub fn resolve_key(&self, provider: &Provider) -> String {
        let raw = provider.api_key.trim();
        if let Some(inner) = raw.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
            crate::server::platform::env::var(inner).unwrap_or_default()
        } else {
            raw.to_string()
        }
    }
}

// ---- writing one key back ----

/// Write `value` at the key path `keys`, **textually**: every level is a direct child, and
/// everything else — order, comments, alignment — is left byte-for-byte alone.
fn write_key(path: &std::path::Path, keys: &[&str], value: &str) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }

    // Walk down. `found` is the deepest level that exists, with its indentation.
    let mut found: Option<(usize, usize)> = None;
    let mut missing: Option<usize> = None;
    for (depth, key) in keys.iter().enumerate() {
        let (from, to) = match found {
            Some((line, _)) => (line + 1, block_end(&lines, line)),
            None => (0, lines.len()),
        };
        let hit = match found {
            None => (from..to).find(|&i| indent(&lines[i]) == 0 && opens_with(&lines[i], key)),
            Some((_, parent_indent)) => find_child(&lines, from, to, parent_indent, key),
        };
        match hit {
            Some(line) => found = Some((line, indent(&lines[line]))),
            None => {
                missing = Some(depth);
                break;
            }
        }
    }

    match missing {
        // Every level exists: rewrite the leaf where it stands, keeping its
        // comment and its line ending.
        None => {
            let (leaf, leaf_indent) = found.expect("the last level was found");
            let comment = trailing_comment(&lines[leaf]).to_string();
            let nl = if lines[leaf].ends_with('\n') { "\n" } else { "" };
            lines[leaf] = format!(
                "{}{}: {value}{comment}{nl}",
                pad(leaf_indent),
                keys[keys.len() - 1]
            );
        }
        // A level is missing: the remaining chain goes in right after its
        // parent, or at the end of the file when even the top level is absent.
        Some(depth) => {
            let (anchor, first_indent) = match found {
                Some((line, parent_indent)) => (Some(line), parent_indent + 2),
                None => (None, 0),
            };
            let last = keys.len() - 1;
            let chain: Vec<String> = keys[depth..]
                .iter()
                .enumerate()
                .map(|(k, key)| {
                    let at = first_indent + 2 * k;
                    if depth + k == last {
                        format!("{}{key}: {value}\n", pad(at))
                    } else {
                        format!("{}{key}:\n", pad(at))
                    }
                })
                .collect();
            match anchor {
                Some(at) => {
                    for (k, line) in chain.into_iter().enumerate() {
                        lines.insert(at + 1 + k, line);
                    }
                }
                None => {
                    if let Some(last_line) = lines.last_mut()
                        && !last_line.ends_with('\n')
                    {
                        last_line.push('\n');
                    }
                    lines.extend(chain);
                }
            }
        }
    }
    write_back(path, &lines)
}

fn write_back(path: &std::path::Path, lines: &[String]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, lines.concat())?;
    Ok(())
}

/// Does this line open the key `key` (the name has to be a whole word, so
/// `api:` matches and `apiKey:` does not)?
fn opens_with(line: &str, key: &str) -> bool {
    let t = line.trim_start();
    t.starts_with(key) && t.as_bytes().get(key.len()) == Some(&b':')
}

/// The indentation the block's own children sit at: the smallest indent deeper than the parent's,
/// so a same-named key several levels down is not mistaken for a direct child.
fn child_indent(lines: &[String], from: usize, to: usize, parent: usize) -> Option<usize> {
    (from..to)
        .filter(|&i| {
            let t = lines[i].trim();
            !t.is_empty() && !t.starts_with('#') && indent(&lines[i]) > parent
        })
        .map(|i| indent(&lines[i]))
        .min()
}

/// The first **direct** child of the block `from..to` that opens the key `key`.
fn find_child(
    lines: &[String],
    from: usize,
    to: usize,
    parent: usize,
    key: &str,
) -> Option<usize> {
    let at = child_indent(lines, from, to, parent)?;
    (from..to).find(|&i| indent(&lines[i]) == at && opens_with(&lines[i], key))
}

/// One past the last line of the block opened at `at`: blank lines, comments and
/// anything indented deeper belong to it.
fn block_end(lines: &[String], at: usize) -> usize {
    let base = indent(&lines[at]);
    let mut i = at + 1;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || indent(&lines[i]) > base {
            i += 1;
        } else {
            break;
        }
    }
    i
}

/// Leading spaces.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn pad(n: usize) -> String {
    " ".repeat(n)
}

/// The trailing `# comment` of a line together with the whitespace before it,
/// or `""`. The newline is not included.
fn trailing_comment(line: &str) -> &str {
    let Some(at) = comment_start(line) else {
        return "";
    };
    let end = line.trim_end_matches('\n').len();
    let start = line[..at]
        .rfind(|c: char| !c.is_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    &line[start..end]
}

/// Byte offset of the line's trailing `#` comment, or `None`.
fn comment_start(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) if c == b'\\' && q == b'"' => i += 1, // skip the escaped byte
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'\'' || c == b'"' => quote = Some(c),
            None if c == b'#' && (i == 0 || b[i - 1].is_ascii_whitespace()) => {
                // A comment marker needs whitespace (or start of line) before it.
                return Some(i);
            }
            None => {}
        }
        i += 1;
    }
    None
}
