use herdcats_bridge::service::{Service, serve};
use std::{path::PathBuf, time::Duration};
fn run() -> std::result::Result<(), &'static str> {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if command.as_deref() == Some("--version-json") || command.as_deref() == Some("--version") {
        if args.next().is_some() {
            return Err("unknown_argument");
        }
        // install.sh (including released installers) compares these bytes exactly;
        // keep the keys in alphabetical order even with preserve_order enabled.
        println!(
            "{}",
            serde_json::json!({
                "agents": ["claude", "codex"],
                "herdr_protocol": 22,
                "name": "herdcats-bridge",
                "protocol": 1,
                "version": env!("CARGO_PKG_VERSION")
            })
        );
        return Ok(());
    }
    if command.as_deref() == Some("hooks") {
        let args: Vec<_> = args.collect();
        let result = match args.as_slice() {
            [action, flag, agent] if flag == "--agent" => {
                match herdcats_bridge::hooks::Agent::parse(agent) {
                    Some(agent) => match action.as_str() {
                        "install" => herdcats_bridge::hooks::install(agent),
                        "status" => herdcats_bridge::hooks::status(agent),
                        "uninstall" => herdcats_bridge::hooks::uninstall(agent),
                        _ => herdcats_bridge::hooks::HooksResult::error("unknown_argument"),
                    },
                    None => herdcats_bridge::hooks::HooksResult::error("invalid_agent"),
                }
            }
            _ => herdcats_bridge::hooks::HooksResult::error("unknown_argument"),
        };
        println!("{}", result.to_json());
        return Ok(());
    }
    if command.as_deref() == Some("codex-config") {
        let sub = args.next();
        match sub.as_deref() {
            Some("daemon-status") => {
                if args.next().is_some() {
                    return Err("usage: herdcats-bridge codex-config daemon-status");
                }
                let res = herdcats_bridge::codex_config::daemon_status();
                println!("{}", res.to_json());
                return Ok(());
            }
            Some("disable-daemon") => {
                let mut stop_running = false;
                for arg in args {
                    if arg == "--stop-running" {
                        stop_running = true;
                    } else {
                        return Err(
                            "usage: herdcats-bridge codex-config disable-daemon [--stop-running]",
                        );
                    }
                }
                let res =
                    herdcats_bridge::codex_config::disable_daemon_with_options(stop_running);
                println!("{}", res.to_json());
                return Ok(());
            }
            _ => {
                return Err(
                    "usage: herdcats-bridge codex-config <disable-daemon [--stop-running]|daemon-status>",
                );
            }
        }
    }
    if command.as_deref() != Some("serve") || args.next().as_deref() != Some("--stdio") {
        return Err(
            "usage: herdcats-bridge serve --stdio [--socket PATH] [--claude-root PATH] [--codex-root PATH] [--poll-ms 50..5000]",
        );
    }
    let mut socket = std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from);
    let mut root = std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".claude/projects"));
    let mut codex_root: Option<PathBuf> = None;
    let mut poll = 300;
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("missing_argument")?;
        match flag.as_str() {
            "--socket" => socket = Some(value.into()),
            "--claude-root" => root = Some(value.into()),
            "--codex-root" => codex_root = Some(value.into()),
            "--poll-ms" => {
                poll = value
                    .parse::<u64>()
                    .ok()
                    .filter(|n| (50..=5000).contains(n))
                    .ok_or("invalid_poll_interval")?
            }
            _ => return Err("unknown_argument"),
        }
    }
    let mut service = Service::new(
        socket.ok_or("socket_required")?,
        root.ok_or("claude_root_required")?,
    );
    if let Some(cr) = codex_root {
        service = service.with_codex_root(cr);
    }
    if let Some(dir) = herdcats_bridge::hooks::resolve_spool_dir() {
        let _ = herdcats_bridge::spool::prune(&dir, std::time::SystemTime::now());
    }
    serve(service, Duration::from_millis(poll)).map_err(|_| "stream_closed")
}
fn main() {
    // Recognize hooks before UTF-8 argument parsing or the normal error path.
    // Hooks never decide permissions and must not emit even a panic diagnostic.
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("hook")) {
        std::panic::set_hook(Box::new(|_| {}));
        let _ = std::panic::catch_unwind(herdcats_bridge::spool::record_cli);
        return;
    }
    if let Err(code) = run() {
        eprintln!("{code}");
        std::process::exit(1);
    }
}
