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
        static func content(reference: String) throws -> Data {
            if reference == imageRef { return imageBlock }
            if reference == unfamiliarRef { return unfamiliarBlock }
            throw AgentBridgeError.unavailable("content_unavailable")
        }
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
                        ], "generation": 1, "entries": entries, "has_more": false, "agent_status": "idle",
                        "unknown_records": ProcessInfo.processInfo.arguments.contains("-hc.timeline-long") ? 2 : 0, "dropped_entries": 0,
                    ],
                ]))
        }
    }
#endif
