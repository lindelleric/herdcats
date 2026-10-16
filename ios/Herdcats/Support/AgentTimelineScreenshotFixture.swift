#if DEBUG && targetEnvironment(simulator)
    import Foundation
    import UIKit

    @MainActor enum AgentTimelineScreenshotFixture {
        private static let imageBlock: Data = {
            let format = UIGraphicsImageRendererFormat(); format.scale = 1
            let image = UIGraphicsImageRenderer(size: CGSize(width: 360, height: 180), format: format).image { context in
                UIColor.systemMint.setFill(); context.fill(CGRect(x: 0, y: 0, width: 360, height: 180))
                ("Session image" as NSString).draw(at: CGPoint(x: 70, y: 70), withAttributes: [.font: UIFont.systemFont(ofSize: 28), .foregroundColor: UIColor.black])
            }
            return try! JSONSerialization.data(withJSONObject: ["type":"image", "source":["type":"base64", "media_type":"image/png", "data":image.pngData()!.base64EncodedString()]])
        }()
        private static let unfamiliarBlock = Data("{\"type\":\"future_block\",\"unexpected\":{\"complete\":\"Every field is preserved\",\"unicode\":\"猫\"}}".utf8)
        private static var imageRef: String { "source:0:\(imageBlock.count)" }
        private static var unfamiliarRef: String { "source:\(imageBlock.count):\(unfamiliarBlock.count)" }
        private static let largeWriteInput: Data = {
            let code = "import SwiftUI\n\nstruct PaywallView: View {\n    var body: some View { Text(\"Pro\") }\n}\n"
                + (0..<320).map { "// Recorded file content line \($0): subscription display and purchase flow." }.joined(separator: "\n")
            return try! JSONSerialization.data(withJSONObject: ["file_path": "/project/PaywallView.swift", "content": code])
        }()
        private static var largeWriteRef: String { "source:1000000:\(largeWriteInput.count)" }
        static func content(reference: String) throws -> Data {
            if reference == largeWriteRef { return largeWriteInput }
            if reference == imageRef { return imageBlock }
            if reference == unfamiliarRef { return unfamiliarBlock }
            throw AgentBridgeError.unavailable("content_unavailable")
        }
        static func historyPage(paneId: String, generation: UInt64) throws -> AgentBridgePayload {
            var rows: [[String: Any]] = [["id": "message:earlier", "kind": "message", "role": "user",
                                          "text": "Earlier conversation message.", "truncated": false]]
            if ScreenshotFixtures.screen == "timeline-history-position" {
                rows += (0..<8).map { index in
                    ["id": "message:older-\(index)", "kind": "message", "role": "assistant",
                     "text": "Earlier update \(index + 1). " + String(repeating: "Reviewed the connection and checked the implementation. ", count: 4),
                     "truncated": false] as [String: Any]
                }
            }
            let data = try JSONSerialization.data(withJSONObject: [
                "generation": generation,
                "identity": ["pane_id": paneId, "terminal_id": "fixture-terminal", "agent": "claude",
                             "native_session_id": "fixture-session"],
                "entries": rows,
                "has_more": false
            ])
            let decoder = JSONDecoder()
            decoder.keyDecodingStrategy = .convertFromSnakeCase
            return try decoder.decode(AgentBridgePayload.self, from: data)
        }
        static func frame(paneId: String, helperInstalled: Bool = false) throws -> AgentBridgeFrame {
            if ScreenshotFixtures.screen == "timeline-disconnected" { throw AgentBridgeError.closed }
            let unavailableCode: String? = switch ScreenshotFixtures.screen {
            case "timeline-unavailable": "unsupported_agent"
            case "timeline-pending": "transcript_unavailable"
            case "timeline-helper-missing" where !helperInstalled: "helper_missing"
            default: nil
            }
            if let unavailableCode {
                return try AgentBridgeFrame.decode(
                    JSONSerialization.data(withJSONObject: [
                        "event": "timeline.unavailable", "epoch": "fixture", "seq": 1,
                        "data": ["pane_id": paneId, "generation": 1, "code": unavailableCode],
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
            if ScreenshotFixtures.screen == "timeline-large-write" {
                entries = [
                    ["id": "message:user", "kind": "message", "role": "user", "text": "Create the paywall view.", "truncated": false],
                    ["id": "tool:large-write", "kind": "tool", "tool_name": "Write", "status": "completed", "text": "Written.",
                     "details": String(decoding: largeWriteInput.prefix(8192), as: UTF8.self), "truncated": true, "content_ref": largeWriteRef],
                    ["id": "message:done", "kind": "message", "role": "assistant", "text": "The paywall view is ready for review.", "truncated": false]
                ]
            }
            if ScreenshotFixtures.screen == "timeline-code-changes" {
                func details(_ value: [String: Any]) -> String {
                    String(decoding: try! JSONSerialization.data(withJSONObject: value), as: UTF8.self)
                }
                entries = [
                    ["id": "message:user", "kind": "message", "role": "user", "text": "Update the triage logic and support records.", "truncated": false],
                    ["id": "tool:edit-triage", "kind": "tool", "tool_name": "Edit", "status": "completed", "text": "Updated.", "truncated": false,
                     "details": details(["file_path": "/project/reply-tool/triage.ts",
                         "old_string": "const skipReady = [];\nfor (const it of items) {\n  if (it.state === \"sendable\") send.push(it);\n  else skipReady.push(it);\n}",
                         "new_string": "const skipReady = [];\nfor (const it of items) {\n  if ((it.comment ?? \"\").trim()) redraft.push(it);\n  else if (it.state === \"sendable\") send.push(it);\n  else skipReady.push(it);\n}"])],
                    ["id": "tool:edit-support", "kind": "tool", "tool_name": "Edit", "status": "completed", "text": "Updated.", "truncated": false,
                     "details": details(["file_path": "/project/support-resolved.json", "old_string": "{\n  \"messages\": 2,\n  \"resolved\": \"2026-05-18\"\n}",
                                         "new_string": "{\n  \"messages\": 4,\n  \"resolved\": \"2026-05-20\"\n}"])],
                    ["id": "task:review", "kind": "tool", "tool_name": "Background task", "status": "completed", "text": "Background review completed", "truncated": false],
                    ["id": "message:done", "kind": "message", "role": "assistant", "text": "The changes are ready for review.", "truncated": false]
                ]
            }
            if ScreenshotFixtures.screen == "timeline-tool-activity",
               let index = entries.firstIndex(where: { $0["id"] as? String == "message:summary" }) {
                entries.insert(contentsOf: [
                    ["id": "tool:push", "kind": "tool", "tool_name": "Bash", "status": "failed",
                     "details": "{\"description\":\"Push changes to pull request #11\",\"command\":\"git push origin feature\"}",
                     "text": "Push rejected.", "truncated": false],
                    ["id": "tool:edit", "kind": "tool", "tool_name": "Edit", "status": "completed",
                     "details": "{\"file_path\":\"/project/README.md\"}", "text": "File updated.", "truncated": false]
                ], at: index)
            }
            if ScreenshotFixtures.screen == "timeline-history-position" {
                entries += (0..<3).map { index in
                    ["id": "message:recent-\(index)", "kind": "message", "role": "assistant",
                     "text": "Recent update \(index + 1). " + String(repeating: "Reviewed the connection and checked the implementation. ", count: 4),
                     "truncated": false] as [String: Any]
                }
            }
            if ProcessInfo.processInfo.arguments.contains("-hc.timeline-long") {
                entries += (0..<12).map { index in
                    ["id": "message:history-\(index)", "kind": "message", "role": "assistant",
                     "text": "Session update \(index + 1). Reviewed the implementation, checked the changed files, and confirmed the result with a focused test run.",
                     "truncated": false] as [String: Any]
                }
            }
            if ScreenshotFixtures.screen == "timeline-rich-content" {
                entries += [
                    ["id":"content:image", "kind":"image", "role":"user", "text":"Image", "truncated":false, "content_ref":imageRef, "record_refs":[imageRef]],
                    ["id":"content:unfamiliar", "kind":"record", "text":"future_block", "details":String(decoding: unfamiliarBlock, as: UTF8.self), "truncated":false, "content_ref":unfamiliarRef, "record_refs":[unfamiliarRef]]
                ]
            }
            return try AgentBridgeFrame.decode(
                JSONSerialization.data(withJSONObject: [
                    "event": "timeline.snapshot", "epoch": "fixture", "seq": 1,
                    "data": [
                        "identity": [
                            "pane_id": paneId, "terminal_id": "fixture-terminal", "agent": "claude",
                            "native_session_id": "fixture-session",
                        ], "generation": 1, "entries": entries, "has_more": ScreenshotFixtures.screen.hasPrefix("timeline-history"), "before": "message:user", "agent_status": "idle",
                        "unknown_records": ProcessInfo.processInfo.arguments.contains("-hc.timeline-long") ? 2 : 0, "dropped_entries": 0,
                    ],
                ]))
        }
    }
#endif
