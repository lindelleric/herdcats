use herdcats_bridge::approval_screen::{OptionKey, parse, parse_claude, parse_codex};
use herdcats_bridge::hooks::Agent;

const CLAUDE: &str = include_str!("../../compatibility/phase1/fixtures/claude-approval-screen.txt");
const CODEX: &str = include_str!("../../compatibility/phase1/fixtures/codex-approval-screen.txt");

#[test]
fn fixtures() {
    let c = parse(Agent::Claude, CLAUDE).unwrap();
    assert_eq!(c.agent, Agent::Claude);
    assert_eq!(c.title, "Bash command");
    assert_eq!(c.detail, ["echo herdcats-probe > probe.txt"]);
    assert!(c.reason_lines.is_empty());
    assert_eq!(c.selected, Some(0));
    let labels = [
        vec!["Yes".to_string()],
        vec![
            format!(
                "Yes, and always allow access to /fixture/work/{}",
                "x".repeat(67)
            ),
            format!("{} from this project", "x".repeat(70)),
        ],
        vec!["Yes, and switch to auto mode · auto mode handles these prompts for you".into()],
        vec!["No".into()],
    ];
    for (i, option) in c.options.iter().enumerate() {
        assert_eq!(option.number, i as u32 + 1);
        assert_eq!(option.label_lines, labels[i]);
        assert_eq!(option.key, OptionKey::Digit(i as u32 + 1));
    }
    assert_eq!(c.options.len(), 4);
    let c = parse(Agent::Codex, CODEX).unwrap();
    assert_eq!(c.agent, Agent::Codex);
    assert_eq!(c.title, "Would you like to run the following command?");
    assert_eq!(c.detail, ["echo herdcats-probe > probe2.txt"]);
    assert_eq!(
        c.reason_lines,
        ["Allow the exact requested command to write probe2.txt?"]
    );
    assert_eq!(c.selected, Some(0));
    assert_eq!(c.options.len(), 3);
    for (i, (label, key)) in [("Yes, proceed", OptionKey::Letter('y')), ("Yes, and don't ask again for commands that start with `echo herdcats-probe > probe2.txt`", OptionKey::Letter('p')), ("No, and tell Codex what to do differently", OptionKey::Escape)].into_iter().enumerate() {
        assert_eq!(c.options[i].number, i as u32 + 1);
        assert_eq!(c.options[i].label_lines, [label]);
        assert_eq!(c.options[i].key, key);
    }
}

#[test]
fn rejects_incomplete_or_ambiguous_screens() {
    for (agent, screen, other) in [
        (Agent::Claude, CLAUDE, CODEX),
        (Agent::Codex, CODEX, CLAUDE),
    ] {
        for invalid in [
            "".to_string(),
            other.to_string(),
            screen.replace("2.", "1."),
            screen.replace("2.", "5."),
            screen
                .lines()
                .filter(|l| !l.contains(". "))
                .collect::<Vec<_>>()
                .join("\n"),
        ] {
            assert!(parse(agent, &invalid).is_none(), "{invalid}");
        }
        let footer = if agent == Agent::Claude {
            "Esc to cancel"
        } else {
            "Press enter to confirm or esc to cancel"
        };
        assert!(parse(agent, screen.split(footer).next().unwrap()).is_none());
    }
    assert!(parse_codex(&CODEX.replace(" (y)", "")).is_none());
    assert!(parse_codex(&CODEX.replace("(y)", "(yes)")).is_none());
}

#[test]
fn ignores_surrounding_output_and_whitespace() {
    for (agent, screen) in [(Agent::Claude, CLAUDE), (Agent::Codex, CODEX)] {
        let padded = format!(
            "unrelated output\nmore output\n\n{}\n\n",
            screen
                .lines()
                .map(|l| format!("{l}  "))
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(parse(agent, &padded), parse(agent, screen));
    }
}

#[test]
fn codex_wrapped_labels_and_no_selection() {
    let screen = CODEX.replace("› 1. Yes, proceed (y)", "  1. Yes,\n     proceed (y)");
    let c = parse_codex(&screen).unwrap();
    assert_eq!(c.options[0].label_lines, ["Yes,", "proceed"]);
    assert_eq!(c.options[0].key, OptionKey::Letter('y'));
    assert_eq!(c.selected, None);
}

#[test]
fn fingerprint_tracks_content_not_selection() {
    for (agent, screen, marker) in [(Agent::Claude, CLAUDE, '❯'), (Agent::Codex, CODEX, '›')] {
        let original = parse(agent, screen).unwrap().fingerprint();
        assert_eq!(
            original,
            parse(agent, &screen.replace(marker, " "))
                .unwrap()
                .fingerprint()
        );
        for changed in [
            screen.replace("Yes", "Absolutely"),
            screen.replace("echo herdcats-probe", "echo changed"),
        ] {
            assert_ne!(original, parse(agent, &changed).unwrap().fingerprint());
        }
    }
    assert!(parse_claude(CODEX).is_none());
}

#[test]
fn claude_ignores_wrapped_tip_and_description_rows() {
    let screen = CLAUDE
        .replace(
            " Tip: auto mode handles these prompts for you — choose \"switch to auto mode\" below",
            " Tip: auto mode handles these prompts for you\n choose \"switch to auto mode\" below",
        )
        .replace(
            "Write probe text to probe.txt",
            "Write probe text\n to probe.txt",
        );
    let dialog = parse_claude(&screen).unwrap();
    let original = parse_claude(CLAUDE).unwrap();
    assert_eq!(dialog.title, "Bash command");
    assert_eq!(dialog.detail, original.detail);
    assert_eq!(dialog.options, original.options);
    assert_eq!(dialog.fingerprint(), original.fingerprint());
    assert_eq!(
        parse_claude(&CLAUDE.replace(
            " echo herdcats-probe > probe.txt",
            " echo herdcats-probe > probe.txt\n\n"
        ))
        .unwrap()
        .detail,
        ["echo herdcats-probe > probe.txt", "", ""]
    );
    assert!(parse_claude(&CLAUDE.replace("Bash command", "Edit file")).is_none());
}

#[test]
fn codex_preserves_command_and_reason_rows_and_rejects_malformed_metadata() {
    let screen = CODEX
        .replace(
            "$ echo herdcats-probe > probe2.txt",
            "$ echo herdcats-
  probe > probe2.txt",
        )
        .replace(
            "Reason: Allow the exact requested command to write probe2.txt?",
            "Reason: Allow the exact requested
  command to write probe2.txt?",
        );
    let dialog = parse_codex(&screen).unwrap();
    assert_eq!(dialog.detail, ["echo herdcats-", "probe > probe2.txt"]);
    assert_eq!(
        dialog.reason_lines,
        ["Allow the exact requested", "command to write probe2.txt?"]
    );
    assert!(
        parse_codex(&CODEX.replace(
            "› 1.",
            "  Reason:
› 1."
        ))
        .is_none()
    );
    assert!(parse_codex(&CODEX.replace("  Environment: local", "  unexplained header")).is_none());
}

#[test]
fn fingerprints_preserve_wrap_boundaries_and_all_row_groups() {
    for (agent, screen, old, new) in [
        (
            Agent::Codex,
            CODEX,
            "echo herdcats-probe > probe2.txt",
            "echo herdcats-\n  probe > probe2.txt",
        ),
        (
            Agent::Codex,
            CODEX,
            "Yes, proceed (y)",
            "Yes, pro\n     ceed (y)",
        ),
        (
            Agent::Codex,
            CODEX,
            "Reason: Allow the exact",
            "Reason: Allow the\n  exact",
        ),
    ] {
        assert_ne!(
            parse(agent, screen).unwrap().fingerprint(),
            parse(agent, &screen.replacen(old, new, 1))
                .unwrap()
                .fingerprint()
        );
    }
}
