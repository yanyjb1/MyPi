//! Find a template, read its YAML tool list, and render its `{{...}}` variables.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::server::agent::tools::Env;

#[derive(Deserialize)]
struct Front {
    tools: Option<Vec<String>>,
}

pub(crate) struct Template {
    pub(crate) body: String,
    pub(crate) tools: Vec<String>,
}

pub(crate) fn load(
    env: &Env,
    name: &str,
    vars: &BTreeMap<String, String>,
    parent_tools: &BTreeSet<String>,
) -> Result<Template> {
    let path = find(env, name)?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read the template {}", path.display()))?;
    let (front, body) = split_front(&text)?;
    let front: Front = serde_yaml::from_str(front).context("invalid template YAML")?;
    let tools = front.tools.context("the template must declare `tools`")?;
    anyhow::ensure!(
        !tools.is_empty(),
        "the template's `tools` must not be empty"
    );
    for tool in &tools {
        if tool == "task" {
            bail!("a template cannot include `task`");
        }
        anyhow::ensure!(
            parent_tools.contains(tool),
            "template tool `{tool}` is not in the parent's roster"
        );
    }
    let body = render(body, vars)?;
    Ok(Template { body, tools })
}

fn find(env: &Env, name: &str) -> Result<PathBuf> {
    let file = if name.ends_with(".md") {
        name.to_string()
    } else {
        format!("{name}.md")
    };
    let project = env
        .workspace
        .root()
        .join(".mypi")
        .join("agents")
        .join(&file);
    if project.is_file() {
        return Ok(project);
    }
    let global = crate::server::platform::env::config_dir()
        .join("agents")
        .join(&file);
    if global.is_file() {
        return Ok(global);
    }
    let raw = Path::new(name);
    let path = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        env.workspace.cwd().join(raw)
    };
    anyhow::ensure!(
        path.is_file(),
        "no template `{name}`; looked at {}, {}, and {}",
        project.display(),
        global.display(),
        path.display()
    );
    Ok(path)
}

fn split_front(text: &str) -> Result<(&str, &str)> {
    let rest = text
        .strip_prefix("---\n")
        .context("the template must start with a `---` YAML block")?;
    let (front, body) = rest
        .split_once("\n---\n")
        .context("the template's YAML block is not closed")?;
    Ok((front, body))
}

fn render(body: &str, vars: &BTreeMap<String, String>) -> Result<String> {
    let mut found = BTreeSet::new();
    let mut rest = body;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            bail!("unclosed `{{{{` in the template");
        };
        let name = after[..end].trim();
        anyhow::ensure!(!name.is_empty(), "empty variable name in the template");
        found.insert(name.to_string());
        rest = &after[end + 2..];
    }
    for name in &found {
        anyhow::ensure!(
            vars.contains_key(name),
            "the template needs a value for `{name}`"
        );
    }
    for name in vars.keys() {
        anyhow::ensure!(
            found.contains(name),
            "variable `{name}` is not used by the template"
        );
    }
    let mut out = body.to_string();
    for name in found {
        let value = vars.get(&name).expect("checked above");
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    Ok(out)
}
