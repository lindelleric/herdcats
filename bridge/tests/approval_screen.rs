use herdcats_bridge::approval_screen::{OptionKey, parse, parse_claude, parse_codex};
use herdcats_bridge::hooks::Agent;

const CLAUDE: &str = include_str!("../../compatibility/phase1/fixtures/claude-approval-screen.txt");
const CLAUDE_COMMAND: &str = "echo herdcats-probe > probe.txt";
const CODEX_COMMAND: &str = "echo herdcats-probe > probe2.txt";

fn fixture_command(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => CLAUDE_COMMAND,
        Agent::Codex => CODEX_COMMAND,
    }
}

const CODEX: &str = include_str!("../../compatibility/phase1/fixtures/codex-approval-screen.txt");

#[test]
fn fixtures() {
    let c = parse(Agent::Claude, CLAUDE, fixture_command(Agent::Claude)).unwrap();
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
    let c = parse(Agent::Codex, CODEX, fixture_command(Agent::Codex)).unwrap();
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
            assert!(
                parse(agent, &invalid, fixture_command(agent)).is_none(),
                "{invalid}"
            );
        }
        let footer = if agent == Agent::Claude {
            "Esc to cancel"
        } else {
            "Press enter to confirm or esc to cancel"
        };
        assert!(
            parse(
                agent,
                screen.split(footer).next().unwrap(),
                fixture_command(agent)
            )
            .is_none()
        );
    }
    assert!(parse_codex(&CODEX.replace(" (y)", ""), CODEX_COMMAND).is_none());
    assert!(parse_codex(&CODEX.replace("(y)", "(yes)"), CODEX_COMMAND).is_none());
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
        assert_eq!(
            parse(agent, &padded, fixture_command(agent)),
            parse(agent, screen, fixture_command(agent))
        );
    }
}

#[test]
fn codex_wrapped_labels_and_no_selection() {
    let screen = CODEX.replace("› 1. Yes, proceed (y)", "  1. Yes,\n     proceed (y)");
    let c = parse_codex(&screen, CODEX_COMMAND).unwrap();
    assert_eq!(c.options[0].label_lines, ["Yes,", "proceed"]);
    assert_eq!(c.options[0].key, OptionKey::Letter('y'));
    assert_eq!(c.selected, None);
}

#[test]
fn canonical_tracks_content_not_selection() {
    for (agent, screen, marker) in [(Agent::Claude, CLAUDE, '❯'), (Agent::Codex, CODEX, '›')] {
        let original = parse(agent, screen, fixture_command(agent))
            .unwrap()
            .canonical();
        let dialog = parse(agent, screen, fixture_command(agent)).unwrap();
        let unselected =
            parse(agent, &screen.replace(marker, " "), fixture_command(agent)).unwrap();
        assert!(dialog.same_dialog(&unselected));
        assert_eq!(original, unselected.canonical());
        for (changed, command) in [
            (
                screen.replace("Yes", "Absolutely"),
                fixture_command(agent).to_string(),
            ),
            (
                screen.replace("echo herdcats-probe", "echo changed"),
                fixture_command(agent).replace("echo herdcats-probe", "echo changed"),
            ),
        ] {
            assert_ne!(
                original,
                parse(agent, &changed, &command).unwrap().canonical()
            );
        }
    }
    assert!(parse_claude(CODEX, CLAUDE_COMMAND).is_none());
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
    let dialog = parse_claude(&screen, CLAUDE_COMMAND).unwrap();
    let original = parse_claude(CLAUDE, CLAUDE_COMMAND).unwrap();
    assert_eq!(dialog.title, "Bash command");
    assert_eq!(dialog.detail, original.detail);
    assert_eq!(dialog.options, original.options);
    assert!(dialog.same_dialog(&original));
    assert_eq!(
        parse_claude(
            &CLAUDE.replace(
                " echo herdcats-probe > probe.txt",
                " echo herdcats-probe > probe.txt\n\n"
            ),
            "echo herdcats-probe > probe.txt\n\n"
        )
        .unwrap()
        .detail,
        ["echo herdcats-probe > probe.txt", "", ""]
    );
    assert!(parse_claude(&CLAUDE.replace("Bash command", "Edit file"), CLAUDE_COMMAND).is_none());
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
    let dialog = parse_codex(&screen, CODEX_COMMAND).unwrap();
    assert_eq!(dialog.detail, ["echo herdcats-", "probe > probe2.txt"]);
    assert_eq!(
        dialog.reason_lines,
        ["Allow the exact requested", "command to write probe2.txt?"]
    );
    assert!(
        parse_codex(
            &CODEX.replace("  Environment: local", "  unexplained header"),
            CODEX_COMMAND
        )
        .is_none()
    );
}

#[test]
fn canonical_preserves_wrap_boundaries_and_all_row_groups() {
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
            parse(agent, screen, fixture_command(agent))
                .unwrap()
                .canonical(),
            parse(agent, &screen.replacen(old, new, 1), fixture_command(agent))
                .unwrap()
                .canonical()
        );
    }
}

#[test]
fn codex_command_rows_never_become_metadata() {
    let screen = CODEX.replace("$ echo herdcats-probe > probe2.txt", "$ echo herdcats-probe > probe2.txt\n  Environment: rm -rf\n  Reason: command text\n  $ second row\n  Reason:");
    let dialog = parse_codex(&screen, "echo herdcats-probe > probe2.txt\nEnvironment: rm -rf\nReason: command text\n$ second row\nReason:").unwrap();
    assert_eq!(
        dialog.detail,
        [
            "echo herdcats-probe > probe2.txt",
            "Environment: rm -rf",
            "Reason: command text",
            "$ second row",
            "Reason:"
        ]
    );
    assert_eq!(
        dialog.reason_lines,
        parse_codex(CODEX, CODEX_COMMAND).unwrap().reason_lines
    );
}

#[test]
fn canonical_includes_option_keys() {
    let original = parse_codex(CODEX, CODEX_COMMAND).unwrap();
    let changed = parse_codex(&CODEX.replace("(y)", "(n)"), CODEX_COMMAND).unwrap();
    assert_eq!(
        original.options[0].label_lines,
        changed.options[0].label_lines
    );
    assert!(!original.same_dialog(&changed));
    for key in [OptionKey::Digit(1), OptionKey::Digit(2), OptionKey::Escape] {
        let mut changed = original.clone();
        changed.options[0].key = key;
        assert!(!original.same_dialog(&changed));
    }
}

#[test]
fn command_indentation_is_preserved_relative_to_dialog() {
    for (agent, fixture, old, command, base_indent) in [
        (
            Agent::Claude,
            CLAUDE,
            " echo herdcats-probe > probe.txt",
            " cat <<EOF\n EOF",
            " ",
        ),
        (
            Agent::Codex,
            CODEX,
            "  $ echo herdcats-probe > probe2.txt",
            "  $ cat <<EOF\n  EOF",
            "  ",
        ),
    ] {
        let screen = fixture.replace(old, command);
        let original = parse(agent, &screen, "cat <<EOF\nEOF").unwrap();
        assert_eq!(original.detail, ["cat <<EOF", "EOF"]);
        let indented = parse(
            agent,
            &screen.replace(
                &format!("\n{base_indent}EOF"),
                &format!("\n{base_indent}  EOF"),
            ),
            "cat <<EOF\n  EOF",
        )
        .unwrap();
        assert_eq!(indented.detail, ["cat <<EOF", "  EOF"]);
        assert_ne!(original.canonical(), indented.canonical());
        assert!(
            parse(
                agent,
                &screen.replace(&format!("\n{base_indent}EOF"), "\nEOF"),
                "cat <<EOF\nEOF"
            )
            .is_none()
        );
    }
    let screen = CODEX.replace("$ echo herdcats-probe > probe2.txt", "$   cat <<EOF\n  EOF");
    assert_eq!(
        parse_codex(&screen, "  cat <<EOF\nEOF").unwrap().detail,
        ["  cat <<EOF", "EOF"]
    );
}

#[test]
fn canonical_includes_codex_environment() {
    let local = parse_codex(CODEX, CODEX_COMMAND).unwrap();
    let sandbox = parse_codex(
        &CODEX.replace("Environment: local", "Environment: sandbox"),
        CODEX_COMMAND,
    )
    .unwrap();
    assert_eq!(local.environment.as_deref(), Some("local"));
    assert_eq!(sandbox.environment.as_deref(), Some("sandbox"));
    assert_ne!(local.canonical(), sandbox.canonical());
    assert_eq!(
        parse_claude(CLAUDE, CLAUDE_COMMAND).unwrap().environment,
        None
    );
}

#[test]
fn hook_command_anchors_dialog_and_rejects_ambiguity() {
    for (agent, screen) in [(Agent::Claude, CLAUDE), (Agent::Codex, CODEX)] {
        assert!(parse(agent, screen, "different command").is_none());
        // Scrollback and the live screen are indistinguishable: fail closed.
        assert!(
            parse(
                agent,
                &format!("{screen}\n{screen}"),
                fixture_command(agent)
            )
            .is_none()
        );
    }
}

#[test]
fn codex_embedded_title_and_prompt_cannot_hide_command_prefix() {
    let command = "cat <<EOF\nWould you like to run the following command?\n$ echo suffix\nEOF";
    let screen = CODEX.replacen(
        "$ echo herdcats-probe > probe2.txt",
        "$ cat <<EOF\n  Would you like to run the following command?\n  $ echo suffix\n  EOF",
        1,
    );
    let dialog = parse_codex(&screen, command).unwrap();
    assert_eq!(dialog.detail, command.lines().collect::<Vec<_>>());
    // The matching full command claims the embedded title before it is visited.
    assert_eq!(dialog.detail.first().map(String::as_str), Some("cat <<EOF"));
}

#[test]
fn claude_embedded_rule_cannot_hide_command_prefix() {
    let command = "cat <<EOF\n╌╌╌╌\necho suffix\nEOF";
    let screen = CLAUDE.replacen(
        " echo herdcats-probe > probe.txt",
        " cat <<EOF\n ╌╌╌╌\n echo suffix\n EOF",
        1,
    );
    let dialog = parse_claude(&screen, command).unwrap();
    assert_eq!(dialog.detail, command.lines().collect::<Vec<_>>());
    assert!(parse_claude(&screen, "echo suffix\nEOF").is_none());
}

#[test]
fn canonical_distinguishes_blank_command_rows() {
    let command = "cat <<EOF\nbody\nEOF";
    let blank_command = "cat <<EOF\n\nbody\nEOF";
    let screen = CODEX.replacen(
        "$ echo herdcats-probe > probe2.txt",
        "$ cat <<EOF\n  body\n  EOF",
        1,
    );
    let blank_screen = screen.replace("  body", "\n  body");
    let plain = parse_codex(&screen, command).unwrap();
    let blank = parse_codex(&blank_screen, blank_command).unwrap();
    assert_eq!(plain.detail, ["cat <<EOF", "body", "EOF"]);
    assert_eq!(blank.detail, ["cat <<EOF", "", "body", "EOF"]);
    assert_ne!(plain.canonical(), blank.canonical());
    assert!(parse_codex(&screen, blank_command).is_none());
    assert!(parse_codex(&blank_screen, command).is_none());
}

#[test]
fn wrapped_commands_match_the_unwrapped_hook() {
    for (agent, screen, command) in [
        (Agent::Claude, CLAUDE, CLAUDE_COMMAND),
        (Agent::Codex, CODEX, CODEX_COMMAND),
    ] {
        let indent = if agent == Agent::Claude { " " } else { "  " };
        for wrapped in [
            format!(
                "echo herdcats-\n{indent}probe > {}",
                if agent == Agent::Claude {
                    "probe.txt"
                } else {
                    "probe2.txt"
                }
            ),
            command.replace(" > ", &format!("\n{indent}> ")),
        ] {
            assert!(parse(agent, &screen.replacen(command, &wrapped, 1), command).is_some());
        }
    }
}

#[test]
fn crafted_screens_have_bounded_work() {
    use std::time::{Duration, Instant};
    let title = "  Would you like to run the following command?";
    let mut codex_rows: Vec<String> = (0..394)
        .map(|i| {
            if i % 2 == 0 {
                title.into()
            } else {
                "  $ unrelated".into()
            }
        })
        .collect();
    codex_rows.extend(
        [
            title,
            "  $ echo target",
            "› 1. Yes (y)",
            "  2. No (esc)",
            "  Press enter to confirm or esc to cancel",
            "pane output",
        ]
        .map(str::to_string),
    );
    assert_eq!(codex_rows.len(), 400);
    let mut expected_detail = vec!["unrelated".to_string()];
    expected_detail.extend(
        codex_rows[2..396]
            .iter()
            .map(|row| row.strip_prefix("  ").unwrap().to_string()),
    );
    let command = expected_detail.join("\n");
    let codex_screen = codex_rows.join("\n");
    assert_eq!(codex_screen.lines().count(), 400);
    let start = Instant::now();
    let codex = parse_codex(&codex_screen, &command).unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(codex.detail, expected_detail);

    let mut claude_rows = vec![" Bash command".to_string(), "╌╌╌╌".into()];
    claude_rows.extend((0..392).map(|_| " ╌╌╌╌".to_string()));
    claude_rows.extend(
        [
            "╌╌╌╌",
            " Do you want to proceed?",
            " ❯ 1. Yes",
            "   2. No",
            "",
            " Esc to cancel",
        ]
        .map(str::to_string),
    );
    assert_eq!(claude_rows.len(), 400);
    let command = vec!["╌╌╌╌"; 392].join("\n");
    let claude_screen = claude_rows.join("\n");
    assert_eq!(claude_screen.lines().count(), 400);
    let start = Instant::now();
    let claude = parse_claude(&claude_screen, &command).unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(claude.detail, vec!["╌╌╌╌"; 392]);
}

#[test]
fn command_matching_preserves_internal_whitespace() {
    for (agent, fixture, old, indent) in [
        (Agent::Claude, CLAUDE, CLAUDE_COMMAND, " "),
        (Agent::Codex, CODEX, CODEX_COMMAND, "  "),
    ] {
        for (screen_command, hook_command) in [
            ("printf '%s' 'a b'", "printf '%s' 'ab'"),
            ("printf '%s' 'ab'", "printf '%s' 'a b'"),
            ("echo a\nb", "echo a  b"),
            ("echo a\nb", "echo a\tb"),
        ] {
            let screen_command = screen_command.replace('\n', &format!("\n{indent}"));
            let screen = fixture.replacen(old, &screen_command, 1);
            assert!(parse(agent, &screen, hook_command).is_none());
        }
        let screen = fixture.replacen(old, &format!("cat <<EOF\n{indent}  EOF"), 1);
        assert!(parse(agent, &screen, "cat <<EOF\nEOF").is_none());
        assert!(parse(agent, &screen, "cat <<EOF\n  EOF").is_some());
        assert!(parse(agent, fixture, &format!("{old}\n \t")).is_some());
    }
}

#[test]
fn canonical_preserves_field_boundaries_and_agent() {
    let mut original = parse_codex(CODEX, CODEX_COMMAND).unwrap();
    original.detail = vec!["ab".into(), "c".into()];
    let mut changed = original.clone();
    changed.detail = vec!["a".into(), "bc".into()];
    assert!(!original.same_dialog(&changed));

    let mut changed = original.clone();
    changed.agent = Agent::Claude;
    assert!(!original.same_dialog(&changed));

    original.options[0].label_lines = vec!["a".into(), "b".into()];
    original.options[1].label_lines = vec!["c".into()];
    let mut changed = original.clone();
    changed.options[0].label_lines = vec!["a".into()];
    changed.options[1].label_lines = vec!["b".into(), "c".into()];
    assert!(!original.same_dialog(&changed));

    let mut changed = original.clone();
    changed.environment = None;
    let mut empty_environment = changed.clone();
    empty_environment.environment = Some(String::new());
    assert!(!changed.same_dialog(&empty_environment));
}

#[test]
fn unrelated_codex_scrollback_does_not_claim_live_dialog() {
    let screen = format!("  Would you like to run the following command?\n  $ other\n{CODEX}");
    assert_eq!(
        parse_codex(&screen, CODEX_COMMAND),
        parse_codex(CODEX, CODEX_COMMAND)
    );
}

#[test]
fn claude_footer_text_in_wrapped_option_is_label_text() {
    let screen = CLAUDE.replace(
        "   3.",
        "      Esc to cancel this option\n      continued label\n   3.",
    );
    let dialog = parse_claude(&screen, CLAUDE_COMMAND).unwrap();
    let original = parse_claude(CLAUDE, CLAUDE_COMMAND).unwrap();
    let mut expected = original.options[1].label_lines.clone();
    expected.extend([
        "Esc to cancel this option".to_string(),
        "continued label".to_string(),
    ]);
    assert_eq!(dialog.options[1].label_lines, expected);
    assert_eq!(dialog.options.len(), original.options.len());
    assert_eq!(dialog.options[2..], original.options[2..]);
}

#[test]
fn codex_footer_text_in_wrapped_option_is_label_text() {
    let screen = CODEX.replace(
        " (p)",
        "\n     Press enter to confirm or esc to cancel\n     continued label (p)",
    );
    let dialog = parse_codex(&screen, CODEX_COMMAND).unwrap();
    let original = parse_codex(CODEX, CODEX_COMMAND).unwrap();
    let mut expected = original.options[1].label_lines.clone();
    expected.extend([
        "Press enter to confirm or esc to cancel".to_string(),
        "continued label".to_string(),
    ]);
    assert_eq!(dialog.options[1].label_lines, expected);
    assert_eq!(dialog.options[1].key, OptionKey::Letter('p'));
    assert_eq!(dialog.options[2], original.options[2]);
}

#[test]
fn footer_rows_require_dialog_indent_and_claude_blank_separator() {
    assert!(
        parse_claude(
            &CLAUDE.replace("\n\n Esc to cancel", "\n Esc to cancel"),
            CLAUDE_COMMAND
        )
        .is_none()
    );
    assert!(
        parse_claude(
            &CLAUDE.replace("\n Esc to cancel", "\n  Esc to cancel"),
            CLAUDE_COMMAND
        )
        .is_none()
    );
    assert!(
        parse_codex(
            &CODEX.replace("\n  Press enter", "\n Press enter"),
            CODEX_COMMAND
        )
        .is_none()
    );
}

// Captured Claude Code 2.1.306 visible screen; generated fixtures stay untouched.
const CLAUDE_LIVE_NOTICE: &str = r##"
 ▐▛███▛█   Claude Code v2.1.306
▝▜██████▀  Opus 5.5 with medium effort · Claude Pro
 ▝▝   ▝▝   /private/tmp/claude-501/-Users-zhiyaochan-Projects-ios-app-herdcats/34eb7120-3b5c-423d-9a9b-f92901911409/scratchpad/live/claude


❯ Use the Bash tool to run exactly this one command and nothing else: python3 -c "open('probe.txt','w').write('one')"

  Ran 1 shell command

⏺ I ran the command and it finished with no errors. It should have written one to probe.txt in the working directory
  (/private/tmp/claude-501/-Users-zhiyaochan-Projects-ios-app-herdcats/34eb7120-3b5c-423d-9a9b-f92901911409/scratchpad/live/claude/probe.txt). I didn't open the file to check, since you asked for
  that one command only.

✻ Cooked for 11s · done 3:50 PM

❯ Use the Bash tool to run exactly: python3 -c "open('probe2.txt','w').write('two')"

  Writing "two" to probe2.txt
  ⎿  $ python3 -c "open('probe2.txt','w').write('two')"

───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 Bash command
 Tip: auto mode handles these prompts for you — choose "switch to auto mode" below
 Write "two" to probe2.txt
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 python3 -c "open('probe2.txt','w').write('two')"
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 This command requires approval

 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don’t ask again for: python3 *
   3. Yes, and switch to auto mode · auto mode handles these prompts for you
   4. No

 Esc to cancel · Tab to amend
"##;

const CLAUDE_LIVE_COMMAND: &str = "python3 -c \"open('probe2.txt','w').write('two')\"";

#[test]
fn claude_live_known_notice_is_retained_in_dialog_identity() {
    let dialog = parse_claude(CLAUDE_LIVE_NOTICE, CLAUDE_LIVE_COMMAND).unwrap();
    assert_eq!(dialog.selected, Some(0));
    assert_eq!(dialog.detail, [CLAUDE_LIVE_COMMAND]);
    assert_eq!(dialog.reason_lines, ["This command requires approval"]);
    let labels = [
        vec!["Yes"],
        vec!["Yes, and don’t ask again for: python3 *"],
        vec!["Yes, and switch to auto mode · auto mode handles these prompts for you"],
        vec!["No"],
    ];
    assert_eq!(dialog.options.len(), 4);
    for (index, (option, labels)) in dialog.options.iter().zip(labels).enumerate() {
        assert_eq!(option.number, index as u32 + 1);
        assert_eq!(option.label_lines, labels);
        assert_eq!(option.key, OptionKey::Digit(index as u32 + 1));
    }
    let without_notice = CLAUDE_LIVE_NOTICE.replace(" This command requires approval", "");
    let plain = parse_claude(&without_notice, CLAUDE_LIVE_COMMAND).unwrap();
    assert!(plain.reason_lines.is_empty());
    assert_ne!(dialog.canonical(), plain.canonical());
    assert!(!dialog.same_dialog(&plain));
    let padded = CLAUDE_LIVE_NOTICE.replace(" This command requires approval", "   This command requires approval   ");
    assert!(dialog.same_dialog(&parse_claude(&padded, CLAUDE_LIVE_COMMAND).unwrap()));
}

#[test]
fn claude_live_unknown_notice_is_still_rejected() {
    for notice in [
        "Unrecognized approval notice",
        "This command requires approval.",
        "This command requires approval from an administrator",
        "This command requires approval\n Unexpected additional row",
    ] {
        let screen = CLAUDE_LIVE_NOTICE.replace("This command requires approval", notice);
        assert!(parse_claude(&screen, CLAUDE_LIVE_COMMAND).is_none(), "{notice}");
    }
}

// Captured Codex 0.163.0 visible screen; generated fixtures stay untouched.
const CODEX_LIVE_BLANKS: &str = r##"
  >_ OpenAI Codex (v0.163.0)
     /private/tmp/claude-501/-Users-zhiyaochan-Projects-ios-app-herdcats/34eb7120-3b5c-423d-9a9b-f92901911409/scratchpad/live/codex

  To get started, describe a task or try one of these commands:

  /init - create an AGENTS.md file with instructions for Codex
  /status - show current session configuration
  /permissions - choose what Codex is allowed to do
  /model - choose what model and reasoning effort to use
  /review - review any changes and find issues


› Run exactly this shell command once, requesting escalated permissions since the sandbox is read-only: python3 -c "open('probe2.txt','w').write('two')"


• Hook failed
  └ hook returned invalid session start JSON output

• I’ll run the command once with escalated permissions, as requested.

• Running python3 -c "open('probe2.txt','w').write('two')"




















  Would you like to run the following command?

  Environment: local

  Reason: Allow this command to write probe2.txt with the contents two?

  $ python3 -c "open('probe2.txt','w').write('two')"


› 1. Yes, proceed (y)
  2. Yes, and don't ask again for commands that start with `python3 -c "open('probe2.txt','w').write('two')"` (p)
  3. No, and tell Codex what to do differently (esc)

  Press enter to confirm or esc to cancel
"##;

const CODEX_LIVE_COMMAND: &str = r#"python3 -c "open('probe2.txt','w').write('two')""#;

#[test]
fn codex_live_blank_separators_preserve_metadata_labels_and_shortcuts() {
    let dialog = parse_codex(CODEX_LIVE_BLANKS, CODEX_LIVE_COMMAND).unwrap();
    assert_eq!(dialog.environment.as_deref(), Some("local"));
    assert_eq!(dialog.reason_lines, ["Allow this command to write probe2.txt with the contents two?"]);
    assert_eq!(dialog.detail, [CODEX_LIVE_COMMAND]);
    assert_eq!(dialog.selected, Some(0));
    assert_eq!(dialog.options.len(), 3);
    let labels = [
        "Yes, proceed",
        r#"Yes, and don't ask again for commands that start with `python3 -c "open('probe2.txt','w').write('two')"`"#,
        "No, and tell Codex what to do differently",
    ];
    for (index, (option, key)) in dialog.options.iter().zip([
        OptionKey::Letter('y'), OptionKey::Letter('p'), OptionKey::Escape,
    ]).enumerate() {
        assert_eq!(option.number, index as u32 + 1);
        assert_eq!(option.label_lines, [labels[index]]);
        assert_eq!(option.key, key);
    }
}

#[test]
fn codex_trims_only_trailing_blank_command_rows() {
    let screen = CODEX_LIVE_BLANKS.replace(
        &format!("  $ {CODEX_LIVE_COMMAND}"),
        "  $ echo first\n  \n  echo second",
    );
    let dialog = parse_codex(&screen, "echo first\n\necho second").unwrap();
    assert_eq!(dialog.detail, ["echo first", "", "echo second"]);
    for command in ["echo first\necho second", "echo first echo second", "echo firstecho second"] {
        assert!(parse_codex(&screen, command).is_none(), "{command:?}");
    }
}

#[test]
fn codex_blank_separators_do_not_allow_unknown_nonblank_rows() {
    for screen in [
        CODEX_LIVE_BLANKS.replace("\n\n\n› 1.", "\n Unknown command suffix\n\n› 1."),
        CODEX_LIVE_BLANKS.replace("\n  Environment:", "\n Unknown header\n  Environment:"),
        CODEX_LIVE_BLANKS.replace("\n  Press enter to confirm", "\n  Unknown footer row\n  Press enter to confirm"),
    ] {
        assert_ne!(screen, CODEX_LIVE_BLANKS);
        assert!(parse_codex(&screen, CODEX_LIVE_COMMAND).is_none());
    }
}
