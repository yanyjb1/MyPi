//! Bash safety classification: a classifier, not a sandbox. Deletes are licensed inside the session
//! workspace and the system temp dir only; other paths, and every system root, are refused.

use std::path::Path;

/// One classified hit. The id IS the message for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleHit {
    /// Stable rule id (e.g. `rm-out-of-zone`, `fork-bomb`).
    pub rule_id: &'static str,
    /// `critical` (blocks) or `high` (warns).
    pub tier: &'static str,
}

/// Classification verdict for one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing matched — execute.
    Allow,
    /// High-tier hits only: execute, but prepend the concerns to the
    /// tool output so the model sees what it just did.
    Warn(Vec<RuleHit>),
    /// At least one critical hit — refuse to spawn.
    Block(Vec<RuleHit>),
}

/// Whether the command targets a path outside both licensed zones (the session workspace **and** the
/// system temp dir, where scratch work and artifacts live).
fn escapes_zone(target: &str, zone: &Path) -> bool {
    // `~` and `$HOME` always escape: the licence never covers the home directory itself. Callers may
    // pass already-lowercased targets, so the home forms match case-insensitively.
    let t = target.to_ascii_lowercase();
    if t == "~"
        || t == "$home"
        || t == "${home}"
        || t.starts_with("~/")
        || t.starts_with("$home/")
        || t.starts_with("${home}/")
    {
        return true;
    }
    let abs = if target.starts_with('/') {
        std::path::PathBuf::from(target)
    } else {
        zone.join(target)
    };
    // Non-existent targets: compare lexically after dropping `.`/`..`.
    // Existing targets: canonicalize so symlinks cannot fake containment.
    let resolved = abs.canonicalize().unwrap_or_else(|_| lexical_abs(&abs));
    // Temp licence is strict containment: `/tmp/scratch` is licensed, `/tmp` itself is not. Both the
    // raw and canonicalized roots are compared, so a symlinked `/tmp` cannot fake containment.
    let tmp = std::env::temp_dir();
    let tmp_canon = tmp.canonicalize().unwrap_or_else(|_| tmp.clone());
    let in_tmp = resolved.starts_with(&tmp) || resolved.starts_with(&tmp_canon);
    let is_tmp_root = resolved == tmp || resolved == tmp_canon;
    !(resolved.starts_with(zone) || (in_tmp && !is_tmp_root))
}

// Lexical absolute path without touching the filesystem: collapse `.` and `..`. Above the root stays
// the root.
fn lexical_abs(p: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::from("/");
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Normalize shell-obfuscated spacing so token matching is reliable: strip quotes, undo `\` escapes,
/// fold `${IFS}`/`$IFS` and whitespace runs into single spaces. Ported from pi_agent_rust.
pub fn normalize(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    // Pending whitespace: set by any separator, flushed only when a real character follows.
    let mut pending_space = false;
    let mut rest = command;

    while !rest.is_empty() {
        // ${IFS} / $IFS evaluate to whitespace at runtime; matched case-insensitively (pre-lowercase).
        let ifs_variants = ["${IFS}", "$IFS"];
        if let Some(r) = ifs_variants.iter().find_map(|v| rest.strip_prefix(v)) {
            pending_space = true;
            rest = r;
            continue;
        }

        let mut chars = rest.chars();
        let Some(mut ch) = chars.next() else { break };

        if ch == '\'' || ch == '"' {
            rest = chars.as_str();
            continue;
        }

        if ch == '\\' {
            let mut peek = chars.clone();
            if let Some(next) = peek.next() {
                if next == '\n' || next == '\r' {
                    rest = peek.as_str();
                    continue;
                }
                chars.next();
                if next.is_ascii_whitespace() {
                    pending_space = true;
                    rest = chars.as_str();
                    continue;
                }
                if next == '\'' || next == '"' {
                    rest = chars.as_str();
                    continue;
                }
                ch = next;
            }
        }

        if ch.is_ascii_whitespace() {
            pending_space = true;
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
        rest = chars.as_str();
    }
    out
}

// ---- token stream analysis ----------------------------------------------

/// Shell separators. They are tokenized as **their own tokens** so that glued
/// forms (`x&&rm`, `a|b`) still expose the verb of every segment.
fn is_sep(c: char) -> bool {
    matches!(c, ';' | '&' | '|' | '(' | ')' | '`')
}

/// Split a command into shell-ish tokens: words, with separators standing alone. Positions are byte
/// offsets, so lowercasing first keeps the two streams index-aligned.
fn tokenize(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if is_sep(c) || c.is_ascii_whitespace() {
            if let Some(st) = start.take() {
                out.push(&s[st..i]);
            }
            if is_sep(c) {
                out.push(&s[i..i + c.len_utf8()]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

fn is_sep_token(t: &str) -> bool {
    t.chars().next().is_some_and(is_sep) && t.chars().count() == 1
}

/// Words that may stand **in front of** the real verb without being it: position is what decides,
/// not the mere presence of the token.
const WRAPPERS: &[&str] = &[
    "sudo", "doas", "env", "nohup", "command", "exec", "builtin", "time", "nice", "ionice",
    "setsid", "stdbuf", "xargs", "busybox", "eval", "sh", "bash", "zsh", "dash",
];

/// The token's text without a path prefix (`/bin/rm` → `rm`).
fn verb_text(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

/// Does token `i` begin a command segment? (i.e. it is a word whose
/// predecessor is a separator, or the very first token)
fn starts_segment(tokens: &[&str], i: usize) -> bool {
    !is_sep_token(tokens[i]) && (i == 0 || is_sep_token(tokens[i - 1]))
}

/// End (exclusive) of the segment that starts at `start`.
fn segment_end(tokens: &[&str], start: usize) -> usize {
    let mut i = start + 1;
    while i < tokens.len() && !is_sep_token(tokens[i]) {
        i += 1;
    }
    i
}

/// Indices of the **verbs**: for every command segment, its first token that is not a wrapper, an
/// assignment or a flag. That is what stops `echo 'rm -rf /'` from reading as an rm.
fn verbs(tokens: &[&str]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if starts_segment(tokens, i) {
            let end = segment_end(tokens, i);
            let mut j = i;
            while j < end {
                let t = tokens[j];
                if WRAPPERS.contains(&t)
                    || t.starts_with('-')
                    || (t.contains('=') && !t.starts_with('-'))
                {
                    j += 1;
                    continue;
                }
                out.push(j);
                break;
            }
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

// ---- rules ---------------------------------------------------------------

/// rm targeting outside the workspace, or any system root. The zone rule applies to *every* rm, not
/// only the recursive forced ones.
fn classify_rm(normalized: &str, zone: &Path) -> Option<RuleHit> {
    let lower = normalized.to_ascii_lowercase();
    let orig: Vec<&str> = tokenize(normalized);
    let tokens: Vec<&str> = tokenize(&lower);
    // The verb has to be in **command position**: `echo 'rm -rf /'` only prints. Wrappers and leading
    // flags do not consume the position, so they cannot launder it.
    let pos = *verbs(&tokens)
        .iter()
        .find(|i| verb_text(tokens[**i]) == "rm")?;
    // Targets: the rest of **this segment** (a following `&& cd /etc` is not an
    // rm argument), minus flags.
    let end = segment_end(&tokens, pos);
    let targets: Vec<&str> = orig[pos + 1..end]
        .iter()
        .copied()
        .filter(|t| *t != "--" && !t.starts_with('-'))
        .collect();
    if targets.is_empty() {
        return None;
    }

    // System roots are illegal no matter what the zone is (all-lowercase
    // spellings; matching here uses the lowercased stream).
    const SYSTEM_ROOTS: &[&str] = &[
        "/", "/*", "/.", "/etc", "/usr", "/var", "/boot", "/bin", "/sbin", "/lib", "/lib64",
        "/opt", "/proc", "/sys", "/dev", "/run", "/srv",
    ];
    for t in &targets {
        let tl = t.to_ascii_lowercase();
        if SYSTEM_ROOTS.contains(&tl.as_str()) || escapes_zone(t, zone) {
            return Some(RuleHit {
                rule_id: "rm-out-of-zone",
                tier: "critical",
            });
        }
    }
    None
}

/// Delete-by-rename bypass: `mv target /dev/null`, or moving out of the zone to a scratch dir, still
/// loses the user's file.
fn classify_mv_out_of_zone(normalized: &str, zone: &Path) -> Option<RuleHit> {
    let lower = normalized.to_ascii_lowercase();
    let orig: Vec<&str> = tokenize(normalized);
    let tokens: Vec<&str> = tokenize(&lower);
    let pos = *verbs(&tokens)
        .iter()
        .find(|i| verb_text(tokens[**i]) == "mv")?;
    let end = segment_end(&tokens, pos);
    // Sources keep their original case — see classify_rm.
    let rest: Vec<&str> = orig[pos + 1..end]
        .iter()
        .copied()
        .filter(|t| !t.starts_with('-'))
        .collect();
    // `mv src... dest`: every src except the last is a source.
    if rest.len() < 2 {
        return None;
    }
    let (srcs, _dest) = rest.split_at(rest.len() - 1);
    for s in srcs {
        if escapes_zone(s, zone) {
            return Some(RuleHit {
                rule_id: "mv-out-of-zone",
                tier: "critical",
            });
        }
    }
    None
}

/// `find … -delete` / `-exec rm` outside the zone is an rm in disguise; `fd`/`fdfind` with `-x` or
/// `-exec` gets the same treatment.
fn classify_bulk_delete(normalized: &str, zone: &Path) -> Option<RuleHit> {
    let lower = normalized.to_ascii_lowercase();
    let orig: Vec<&str> = tokenize(normalized);
    let tokens: Vec<&str> = tokenize(&lower);
    // Same anti-laundering and same command-position rule as rm: `find` must be
    // the verb of its segment, not a word inside someone else's argument.
    let pos = *verbs(&tokens).iter().find(|i| {
        let v = verb_text(tokens[**i]);
        v == "find" || v == "fd" || v == "fdfind"
    })?;
    let end = segment_end(&tokens, pos);
    // Search roots keep their original case (see classify_rm); actions are matched on the
    // lowercased stream.
    let rest: Vec<&str> = orig[pos + 1..end].to_vec();
    let actions_lower: Vec<&str> = tokens[pos + 1..end].to_vec();
    // fd takes the search root as a positional (default `.`), find takes start-point(s) after the path
    // list. Either way the roots are the positionals before any flag.
    let split = rest
        .iter()
        .position(|t| t.starts_with('-'))
        .unwrap_or(rest.len());
    let roots: Vec<&str> = rest[..split].to_vec();
    let root_escapes = roots
        .iter()
        .any(|r| escapes_zone(r, zone) || *r == "~" || r.starts_with("~/"));
    let delete_action = actions_lower
        .iter()
        .any(|t| matches!(*t, "-delete" | "-execrm" | "-execdirrm" | "-xrm"));
    // find -exec rm ... : look for the exec + rm pair across tokens.
    let exec_rm = actions_lower.windows(2).any(|w| {
        (w[0] == "-exec" || w[0] == "-execdir" || w[0] == "-x")
            && (w[1] == "rm" || w[1] == "rm," || w[1].starts_with("rm,"))
    }) || actions_lower
        .iter()
        .any(|t| t.starts_with("-exec") && t.contains("rm"));
    if root_escapes && (delete_action || exec_rm) {
        return Some(RuleHit {
            rule_id: "bulk-delete-out-of-zone",
            tier: "critical",
        });
    }
    if !root_escapes && (delete_action || exec_rm) {
        // Root-scoped deletes inside the zone are licensed; nothing to flag.
        return None;
    }
    None
}

fn hit(rule_id: &'static str) -> RuleHit {
    RuleHit {
        rule_id,
        tier: "critical",
    }
}

/// Device-level writes: dd to a device node, mkfs*, fdisk.
fn classify_device_write(lower: &str) -> bool {
    let dd = lower.contains("dd ") && lower.contains("of=/dev/");
    let mkfs = lower.starts_with("mkfs") || lower.contains(" mkfs") || lower.contains(";mkfs");
    let fdisk = lower.starts_with("fdisk") || lower.contains(" fdisk") || lower.contains(";fdisk");
    dd || mkfs || fdisk
}

/// Classic `:(){ :|:& };:` and interpreter fork loops.
fn classify_fork_bomb(lower: &str) -> bool {
    let classic = lower.contains(":(){ :|:&");
    let lang = (lower.contains("perl") || lower.contains("python") || lower.contains("ruby"))
        && lower.contains("fork")
        && lower.contains("while")
        && lower.contains('&');
    classic || lang
}

/// Disk shredders and dd zero/urandom fills.
fn classify_disk_wipe(lower: &str) -> bool {
    let shred = lower.starts_with("shred") || lower.contains(" shred") || lower.contains(";shred");
    let wipefs =
        lower.starts_with("wipefs") || lower.contains(" wipefs") || lower.contains(";wipefs");
    let dd_zero = lower.contains("dd ") && lower.contains("if=/dev/zero");
    let dd_rand = lower.contains("dd ") && lower.contains("if=/dev/urandom");
    shred || wipefs || dd_zero || dd_rand
}

/// Reverse shells: /dev/tcp redirection, nc -e, python socket+connect+sh.
fn classify_reverse_shell(lower: &str) -> bool {
    let bash_rev = lower.contains("/dev/tcp/") && lower.contains("bash");
    let nc_rev = (lower.contains("nc ") || lower.contains("ncat ") || lower.contains("netcat "))
        && lower.contains("-e ");
    // A socket shell needs the **call shapes**, not the bare words: substring matching fired on
    // ordinary prose ("socket connect in push" contains all three).
    let py_rev = lower.contains("socket")
        && (lower.contains("connect(") || lower.contains("create_connection"))
        && (lower.contains("system(")
            || lower.contains("pty.spawn")
            || lower.contains("/bin/sh")
            || lower.contains("os.exec"));
    bash_rev || nc_rev || py_rev
}

/// Download-and-run: curl/wget piped into a shell, `eval "$(curl …)"`, `source <(curl …)`. High tier —
/// occasionally legitimate for setup.
fn classify_pipe_to_shell(lower: &str) -> bool {
    const SHELLS: &[&str] = &[
        "| sh",
        "| bash",
        "|sh",
        "|bash",
        "| /bin/sh",
        "| /bin/bash",
        "|/bin/sh",
        "|/bin/bash",
        "| /usr/bin/sh",
        "| /usr/bin/bash",
        "|/usr/bin/sh",
        "|/usr/bin/bash",
        "| /usr/local/bin/sh",
        "| /usr/local/bin/bash",
        "|/usr/local/bin/sh",
        "|/usr/local/bin/bash",
    ];
    let dl = lower.contains("curl ") || lower.contains("wget ");
    let piped = SHELLS.iter().any(|p| lower.contains(p));
    let eval_forms = [
        "eval \"$(curl ",
        "eval \"$(wget ",
        "eval '$(curl ",
        "eval '$(wget ",
        "eval $(curl ",
        "eval $(wget ",
        "source <(curl ",
        "source <(wget ",
        "bash -c \"$(curl ",
        "bash -c \"$(wget ",
        "bash -c '$(curl ",
        "bash -c '$(wget ",
        "sh -c \"$(curl ",
        "sh -c \"$(wget ",
        "sh -c '$(curl ",
        "sh -c '$(wget ",
    ];
    (dl && piped) || eval_forms.iter().any(|p| lower.contains(p))
}

/// kill of PID 1 / init/systemd/sshd; killall.
fn classify_process_termination(lower: &str) -> bool {
    let pid1 = lower.contains("kill") && (lower.contains(" 1 ") || lower.ends_with(" 1"));
    let force = lower.contains("kill -9") || lower.contains("kill -kill");
    let critical = lower.contains("pkill")
        && (lower.contains("init") || lower.contains("systemd") || lower.contains("sshd"));
    // Command position again: `killall` is a verb, not a word being printed.
    let tokens: Vec<&str> = tokenize(lower);
    let killall = verbs(&tokens)
        .iter()
        .any(|v| verb_text(tokens[*v]) == "killall");
    (pid1 && force) || critical || killall
}

/// Writing to /etc/passwd|shadow|sudoers|sshd_config. Direction matters: `cp /etc/passwd /tmp/x` only
/// reads, while `cp /tmp/x /etc/passwd` overwrites. Each shape is checked where it can write.
fn classify_credential_write(lower: &str) -> bool {
    const FILES: &[&str] = &[
        "/etc/passwd",
        "/etc/shadow",
        "/etc/sudoers",
        "/etc/ssh/sshd_config",
    ];
    // A token names a protected file when it *is* one, possibly glued to a redirection
    // (`>/etc/passwd`). `/etc/passwd.bak` and paths inside longer arguments stay allowed.
    let protected = |t: &str| {
        let t = t.trim_start_matches('>');
        FILES.contains(&t)
    };

    let tokens: Vec<&str> = tokenize(lower);
    for (i, raw) in tokens.iter().enumerate() {
        let t = raw.trim_start_matches('>');
        if !protected(t) {
            continue;
        }
        // Redirection target: the path itself carries `>`, or the previous
        // token is the operator (`echo x > /etc/passwd`).
        if raw.len() != t.len() || (i > 0 && tokens[i - 1].ends_with('>')) {
            return true;
        }
        // `tee /etc/shadow`: the target is tee's first non-flag argument, so `tee -a …` counts too.
        if let Some(verb) = (0..i)
            .rev()
            .find(|j| !tokens[*j].starts_with('-'))
            .filter(|j| starts_segment(&tokens, *j))
            && verb_text(tokens[verb]) == "tee"
        {
            return true;
        }
    }

    // `cp` / `mv` / `sed -i`, resolved per segment so the verb is real.
    for v in verbs(&tokens) {
        let verb = verb_text(tokens[v]);
        let end = segment_end(&tokens, v);
        let args: Vec<&str> = tokens[v + 1..end]
            .iter()
            .copied()
            .filter(|t| !t.starts_with('-'))
            .collect();
        match verb {
            // Only the destination is written.
            "cp" => {
                if args.last().is_some_and(|d| protected(d)) {
                    return true;
                }
            }
            // Both sides: the destination is written, the source's directory
            // loses the file.
            "mv" => {
                if args.iter().any(|a| protected(a)) {
                    return true;
                }
            }
            // In-place edit: any argument can be the file.
            "sed"
                if tokens[v + 1..end].iter().any(|t| t.starts_with("-i"))
                    && args.iter().any(|a| protected(a)) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Shutdown family: the agent has no business turning the machine off. The verb must be in command
/// position — a document merely mentioning "shutdown" is not a shutdown.
fn classify_system_shutdown(lower: &str) -> bool {
    let tokens: Vec<&str> = tokenize(lower);
    for v in verbs(&tokens) {
        // A shutdown reading a pipe is meaningless, and that is exactly what a quoted alternation
        // becomes: `rg 'shutdown|reboot'` looks like `rg shutdown | reboot`. Skip that segment.
        if v > 0 && tokens[v - 1] == "|" {
            continue;
        }
        let verb = verb_text(tokens[v]);
        if ["shutdown", "reboot", "halt", "poweroff"].contains(&verb) {
            return true;
        }
        // `init 0` / `init 6` — the runlevel forms.
        if verb == "init" {
            let end = segment_end(&tokens, v);
            if tokens[v + 1..end].iter().any(|t| *t == "0" || *t == "6") {
                return true;
            }
        }
    }
    false
}

/// Broad permission grants — high tier.
fn classify_permission_escalation(lower: &str) -> bool {
    let broad = lower.contains("chmod")
        && (lower.contains("777") || lower.contains("a+rwx") || lower.contains("o+w"));
    let suid = lower.contains("chmod") && (lower.contains("+s") || lower.contains("4755"));
    broad || suid
}

// ---- entry point ---------------------------------------------------------

/// Classify one command: critical hits block, high hits warn. The zone rules (rm / mv / find-fd bulk
/// delete) are always critical. `zone` MUST be canonicalized by the caller.
pub(crate) fn classify(command: &str, zone: &Path) -> Verdict {
    let normalized = normalize(command);
    let lower = normalized.to_ascii_lowercase();
    let mut hits = Vec::new();

    // Zone rules (critical) — need the un-lowercased path for canonicalize.
    if let Some(h) = classify_rm(&normalized, zone) {
        hits.push(h);
    }
    if let Some(h) = classify_mv_out_of_zone(&normalized, zone) {
        hits.push(h);
    }
    if let Some(h) = classify_bulk_delete(&normalized, zone) {
        hits.push(h);
    }

    // Disaster rules (critical tier).
    if classify_device_write(&lower) {
        hits.push(hit("device-write"));
    }
    if classify_fork_bomb(&lower) {
        hits.push(hit("fork-bomb"));
    }
    if classify_disk_wipe(&lower) {
        hits.push(hit("disk-wipe"));
    }
    if classify_reverse_shell(&lower) {
        hits.push(hit("reverse-shell"));
    }
    if classify_process_termination(&lower) {
        hits.push(hit("process-termination"));
    }
    if classify_credential_write(&lower) {
        hits.push(hit("credential-write"));
    }

    // Critical additions.
    if classify_system_shutdown(&lower) {
        hits.push(hit("system-shutdown"));
    }
    if classify_permission_escalation(&lower) {
        hits.push(hit("permission-escalation"));
    }

    // High tier.
    let mut high = Vec::new();
    if classify_pipe_to_shell(&lower) {
        high.push(RuleHit {
            rule_id: "pipe-to-shell",
            tier: "high",
        });
    }

    if hits.iter().any(|h| h.tier == "critical") {
        hits.extend(high);
        return Verdict::Block(hits);
    }
    if high.is_empty() {
        if hits.is_empty() {
            Verdict::Allow
        } else {
            // Unreachable today (all current rules are critical/high) but
            // kept explicit so future low-tier rules cannot silently block.
            Verdict::Allow
        }
    } else {
        high.extend(hits);
        Verdict::Warn(high)
    }
}
