use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, TableLike, Value};

#[derive(Debug, PartialEq, Eq)]
pub enum CodexConfigResult {
    Changed { backup: Option<String> },
    AlreadyDisabled,
    CodexNotInstalled,
    Error(&'static str),
}

impl CodexConfigResult {
    pub fn to_json(&self) -> String {
        match self {
            CodexConfigResult::Changed { backup: Some(b) } => {
                format!(r#"{{"status":"changed","backup":"{b}"}}"#)
            }
            CodexConfigResult::Changed { backup: None } => r#"{"status":"changed"}"#.to_string(),
            CodexConfigResult::AlreadyDisabled => r#"{"status":"already_disabled"}"#.to_string(),
            CodexConfigResult::CodexNotInstalled => {
                r#"{"status":"codex_not_installed"}"#.to_string()
            }
            CodexConfigResult::Error(code) => {
                format!(r#"{{"error":"{code}"}}"#)
            }
        }
    }
}

pub fn resolve_codex_dir() -> Option<PathBuf> {
    resolve_codex_dir_from(std::env::var_os("CODEX_HOME"), std::env::var_os("HOME"))
}

pub fn resolve_codex_dir_from(
    codex_home: Option<impl AsRef<Path>>,
    home: Option<impl AsRef<Path>>,
) -> Option<PathBuf> {
    if let Some(val) = codex_home {
        let path = val.as_ref();
        if !path.as_os_str().is_empty() && path.is_absolute() {
            return Some(path.to_path_buf());
        }
    }
    if let Some(val) = home {
        let path = val.as_ref();
        if !path.as_os_str().is_empty() && path.is_absolute() {
            return Some(path.join(".codex"));
        }
    }
    None
}

pub fn utc_timestamp_string() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let days = (secs / 86400) as i64;
    let day_secs = (secs % 86400) as u32;
    let hour = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let sec = day_secs % 60;
    let (year, mon, day) = crate::codex::civil_from_days(days);
    format!("{year:04}{mon:02}{day:02}T{hour:02}{min:02}{sec:02}Z")
}

pub fn disable_daemon() -> CodexConfigResult {
    let codex_dir = match resolve_codex_dir() {
        Some(d) => d,
        None => return CodexConfigResult::CodexNotInstalled,
    };
    disable_daemon_in_dir(&codex_dir)
}

pub fn disable_daemon_in_dir(codex_dir: &Path) -> CodexConfigResult {
    if !codex_dir.is_dir() {
        return CodexConfigResult::CodexNotInstalled;
    }
    let config_path = codex_dir.join("config.toml");
    match fs::symlink_metadata(&config_path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return CodexConfigResult::Error("config_symlink");
            }
            if !meta.is_file() {
                return CodexConfigResult::Error("write_failed");
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // Missing config.toml: create with mode 0600
            let write_res: std::io::Result<()> = (|| {
                use std::io::Write;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    let mut file = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&config_path)?;
                    file.write_all(b"[features]\ndaemon_auto_start = false\n")?;
                    file.sync_all()?;
                }
                #[cfg(not(unix))]
                {
                    fs::write(&config_path, b"[features]\ndaemon_auto_start = false\n")?;
                }
                Ok(())
            })();
            if write_res.is_err() {
                return CodexConfigResult::Error("write_failed");
            }
            return CodexConfigResult::Changed { backup: None };
        }
        Err(_) => return CodexConfigResult::Error("write_failed"),
    }

    let orig_metadata = match fs::symlink_metadata(&config_path) {
        Ok(m) => m,
        Err(_) => return CodexConfigResult::Error("write_failed"),
    };
    #[cfg(unix)]
    let orig_mode = {
        use std::os::unix::fs::PermissionsExt;
        orig_metadata.permissions().mode()
    };

    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return CodexConfigResult::Error("write_failed"),
    };

    let mut doc: DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(_) => return CodexConfigResult::Error("config_invalid"),
    };

    if let Some(item) = doc.get("features") {
        if !item.is_table_like() {
            return CodexConfigResult::Error("features_not_table");
        }
        if let Some(tbl) = item.as_table_like() {
            if tbl.get("daemon_auto_start").and_then(|v| v.as_bool()) == Some(false) {
                return CodexConfigResult::AlreadyDisabled;
            }
        }
    }

    if doc.get("features").is_none() {
        let mut t = toml_edit::Table::new();
        t.insert("daemon_auto_start", toml_edit::value(false));
        doc.insert("features", Item::Table(t));
    } else {
        let feat = doc.get_mut("features").unwrap();
        let tbl = feat.as_table_like_mut().unwrap();
        if let Some(item) = tbl.get_mut("daemon_auto_start") {
            match item.as_value_mut() {
                Some(val) if !val.is_array() && !val.is_inline_table() => {
                    let decor = val.decor().clone();
                    *val = Value::from(false);
                    *val.decor_mut() = decor;
                }
                _ => return CodexConfigResult::Error("daemon_setting_unsupported"),
            }
        } else {
            tbl.insert("daemon_auto_start", toml_edit::value(false));
        }
    }

    let timestamp = utc_timestamp_string();
    let mut backup_name = format!("config.toml.bak.herdcats-{timestamp}");
    let mut backup_path = codex_dir.join(&backup_name);
    let mut counter = 1;
    while backup_path.exists() {
        backup_name = format!("config.toml.bak.herdcats-{timestamp}_{counter}");
        backup_path = codex_dir.join(&backup_name);
        counter += 1;
    }

    if fs::copy(&config_path, &backup_path).is_err() {
        return CodexConfigResult::Error("write_failed");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&backup_path, fs::Permissions::from_mode(orig_mode));
    }

    let temp_name = format!(".config.toml.tmp.herdcats-{}", std::process::id());
    let temp_path = codex_dir.join(&temp_name);
    let write_op: std::io::Result<()> = (|| {
        use std::io::Write;
        #[cfg(unix)]
        let mut file = {
            use std::os::unix::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(orig_mode)
                .open(&temp_path)?
        };
        #[cfg(not(unix))]
        let mut file = fs::File::create(&temp_path)?;

        file.write_all(doc.to_string().as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, &config_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&config_path, fs::Permissions::from_mode(orig_mode));
        }
        Ok(())
    })();

    if write_op.is_err() {
        let _ = fs::remove_file(&temp_path);
        return CodexConfigResult::Error("write_failed");
    }

    let reparsed_content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => {
            let _ = fs::copy(&backup_path, &config_path);
            return CodexConfigResult::Error("verification_failed");
        }
    };
    let reparsed_doc: DocumentMut = match reparsed_content.parse() {
        Ok(d) => d,
        Err(_) => {
            let _ = fs::copy(&backup_path, &config_path);
            return CodexConfigResult::Error("verification_failed");
        }
    };
    let orig_doc: DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(_) => {
            let _ = fs::copy(&backup_path, &config_path);
            return CodexConfigResult::Error("verification_failed");
        }
    };

    if !verify_config(&orig_doc, &reparsed_doc) {
        let _ = fs::copy(&backup_path, &config_path);
        return CodexConfigResult::Error("verification_failed");
    }

    CodexConfigResult::Changed {
        backup: Some(backup_name),
    }
}

fn verify_config(orig: &DocumentMut, reparsed: &DocumentMut) -> bool {
    let feat_rep = match reparsed.get("features").and_then(|it| it.as_table_like()) {
        Some(t) => t,
        None => return false,
    };
    if feat_rep.get("daemon_auto_start").and_then(|v| v.as_bool()) != Some(false) {
        return false;
    }

    let orig_keys: std::collections::BTreeSet<_> = orig.iter().map(|(k, _)| k).collect();
    let rep_keys: std::collections::BTreeSet<_> = reparsed.iter().map(|(k, _)| k).collect();
    let all_keys: std::collections::BTreeSet<_> = orig_keys.union(&rep_keys).copied().collect();

    for k in all_keys {
        if k == "features" {
            match orig.get("features") {
                None => {
                    let keys: Vec<_> = feat_rep.iter().map(|(k, _)| k).collect();
                    if keys != vec!["daemon_auto_start"] {
                        return false;
                    }
                }
                Some(orig_feat_item) => {
                    let orig_feat = match orig_feat_item.as_table_like() {
                        Some(t) => t,
                        None => return false,
                    };
                    let f_orig_keys: std::collections::BTreeSet<_> =
                        orig_feat.iter().map(|(k, _)| k).collect();
                    let f_rep_keys: std::collections::BTreeSet<_> =
                        feat_rep.iter().map(|(k, _)| k).collect();
                    let f_all_keys: std::collections::BTreeSet<_> =
                        f_orig_keys.union(&f_rep_keys).copied().collect();
                    for fk in f_all_keys {
                        if fk == "daemon_auto_start" {
                            continue;
                        }
                        match (orig_feat.get(fk), feat_rep.get(fk)) {
                            (Some(ia), Some(ib)) => {
                                if !items_equal(ia, ib) {
                                    return false;
                                }
                            }
                            _ => return false,
                        }
                    }
                }
            }
        } else {
            match (orig.get(k), reparsed.get(k)) {
                (Some(ia), Some(ib)) => {
                    if !items_equal(ia, ib) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }
    true
}

fn items_equal(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (Item::None, Item::None) => true,
        (Item::Value(va), Item::Value(vb)) => values_equal(va, vb),
        (Item::Table(ta), Item::Table(tb)) => tables_equal(ta, tb),
        (Item::ArrayOfTables(aa), Item::ArrayOfTables(ab)) => {
            if aa.len() != ab.len() {
                return false;
            }
            aa.iter()
                .zip(ab.iter())
                .all(|(ta, tb)| tables_equal(ta, tb))
        }
        _ => {
            if let (Some(ta), Some(tb)) = (a.as_table_like(), b.as_table_like()) {
                table_likes_equal(ta, tb)
            } else {
                false
            }
        }
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(sa), Value::String(sb)) => sa.value() == sb.value(),
        (Value::Integer(ia), Value::Integer(ib)) => ia.value() == ib.value(),
        (Value::Float(fa), Value::Float(fb)) => {
            let va = *fa.value();
            let vb = *fb.value();
            va == vb || (va.is_nan() && vb.is_nan())
        }
        (Value::Boolean(ba), Value::Boolean(bb)) => ba.value() == bb.value(),
        (Value::Datetime(da), Value::Datetime(db)) => da.value() == db.value(),
        (Value::Array(aa), Value::Array(ab)) => {
            if aa.len() != ab.len() {
                return false;
            }
            aa.iter()
                .zip(ab.iter())
                .all(|(va, vb)| values_equal(va, vb))
        }
        (Value::InlineTable(ia), Value::InlineTable(ib)) => table_likes_equal(ia, ib),
        _ => false,
    }
}

fn tables_equal(a: &toml_edit::Table, b: &toml_edit::Table) -> bool {
    table_likes_equal(a, b)
}

fn table_likes_equal(a: &dyn TableLike, b: &dyn TableLike) -> bool {
    let keys_a: std::collections::BTreeSet<_> = a.iter().map(|(k, _)| k).collect();
    let keys_b: std::collections::BTreeSet<_> = b.iter().map(|(k, _)| k).collect();
    if keys_a != keys_b {
        return false;
    }
    for k in keys_a {
        match (a.get(k), b.get(k)) {
            (Some(ia), Some(ib)) => {
                if !items_equal(ia, ib) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}
