#if DEBUG && targetEnvironment(simulator)
    import Foundation

    enum AgentTimelineScreenshotFixture {
        static func frame(paneId: String, helperInstalled: Bool = false) throws -> AgentBridgeFrame {
            if ScreenshotFixtures.screen == "timeline-disconnected" { throw AgentBridgeError.closed }
            if ScreenshotFixtures.screen == "timeline-unavailable" || (ScreenshotFixtures.screen == "timeline-helper-missing" && !helperInstalled) {
                return try AgentBridgeFrame.decode(
                    JSONSerialization.data(withJSONObject: [
                        "event": "timeline.unavailable", "epoch": "fixture", "seq": 1,
                        "data": ["pane_id": paneId, "generation": 1, "code": ScreenshotFixtures.screen == "timeline-helper-missing" ? "helper_missing" : "unsupported_agent"],
                    ]))
            }
            var entries: [[String: Any]] =
                ScreenshotFixtures.screen == "timeline-empty"
                ? []
                : [
                    [
                        "id": "message:user", "kind": "message", "role": "user",
                        "text": "Review the connection guidance and make it clearer in light mode.",
                        "truncated": false,
                    ],
                    [
                        "id": "message:assistant", "kind": "message", "role": "assistant",
                        "text":
                            "I checked the onboarding flow. The new guidance explains **host key verification** before connecting and keeps the SSH steps easy to follow.",
                        "truncated": false,
                    ],
                    [
                        "id": "tool:read", "kind": "tool", "tool_name": "Read", "status": "completed",
                        "text": "Connection guidance reviewed.\n24 tests passed.",
                        "details":
                            "{\"path\":\"ios/Herdcats/Features/Connect/HostKeyVerificationView.swift\"}",
                        "truncated": false,
                    ],
                    [
                        "id": "message:summary", "kind": "message", "role": "assistant",
                        "text":
                            "The changes are ready for review.\n\nUse Terminal to send a message or respond to the current request.",
                        "truncated": false,
                    ],
                ]
            if ProcessInfo.processInfo.arguments.contains("-hc.timeline-long") {
                entries += (0..<12).map { index in
                    ["id": "message:history-\(index)", "kind": "message", "role": "assistant",
                     "text": "Session update \(index + 1). Reviewed the implementation, checked the changed files, and confirmed the result with a focused test run.",
                     "truncated": false] as [String: Any]
                }
            }
            return try AgentBridgeFrame.decode(
                JSONSerialization.data(withJSONObject: [
                    "event": "timeline.snapshot", "epoch": "fixture", "seq": 1,
                    "data": [
                        "identity": [
                            "pane_id": paneId, "terminal_id": "fixture-terminal", "agent": "claude",
                            "native_session_id": "fixture-session",
                        ], "generation": 1, "entries": entries, "has_more": false, "agent_status": "idle",
                        "unknown_records": 0, "dropped_entries": 0,
                    ],
                ]))
        }
    }
#endif
