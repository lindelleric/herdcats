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
    assert_eq!(c.reason, None);
    assert_eq!(c.selected, Some(0));
    let labels = [
        "Yes".to_string(),
        format!(
            "Yes, and always allow access to /fixture/work/{} {} from this project",
            "x".repeat(67),
            "x".repeat(70)
        ),
        "Yes, and switch to auto mode · auto mode handles these prompts for you".into(),
        "No".into(),
    ];
    for (i, option) in c.options.iter().enumerate() {
        assert_eq!(option.number, i as u32 + 1);
        assert_eq!(option.label, labels[i]);
        assert_eq!(option.key, OptionKey::Digit(i as u32 + 1));
    }
    assert_eq!(c.options.len(), 4);
    let c = parse(Agent::Codex, CODEX).unwrap();
    assert_eq!(c.agent, Agent::Codex);
    assert_eq!(c.title, "Would you like to run the following command?");
    assert_eq!(c.detail, ["echo herdcats-probe > probe2.txt"]);
    assert_eq!(
        c.reason.as_deref(),
        Some("Allow the exact requested command to write probe2.txt?")
    );
    assert_eq!(c.selected, Some(0));
    assert_eq!(c.options.len(), 3);
    for (i, (label, key)) in [("Yes, proceed", OptionKey::Letter('y')), ("Yes, and don't ask again for commands that start with `echo herdcats-probe > probe2.txt`", OptionKey::Letter('p')), ("No, and tell Codex what to do differently", OptionKey::Escape)].into_iter().enumerate() {
        assert_eq!(c.options[i].number, i as u32 + 1);
        assert_eq!(c.options[i].label, label);
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
    assert_eq!(c.options[0].label, "Yes, proceed");
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
