# Fork notes (lindelleric/herdcats)

Personal fork of [zhiyao/herdcats](https://github.com/zhiyao/herdcats), built
from source on Linux with xtool instead of trusting the TestFlight binary.

Goal: one native iPhone app for everything terminal. Primarily driving
[herdr](https://herdr.dev) to run **pi** as the agent harness, but also
arbitrary terminal work (replacing Termius). Claude, Codex, Agy, Cursor and
quota features are not needed.

## Layout

- Local clone: `~/projects/herdcats`
- Remotes: `origin` = `lindelleric/herdcats` (public, since GitHub forks of
  public repos are always public), `upstream` = `zhiyao/herdcats`
- Working branch: `xtool`
- Bundle ID: `se.lindell.herdcats`
- Target SSH host: `hermes` (192.168.1.131)

## Build and install

```sh
cd ~/projects/herdcats/ios
xtool dev build            # build only
xtool dev run --usb        # build, sign, install on the iPhone over USB
xtool dev run --usb -c release
```

First full build takes about 1.5 minutes, incremental builds are a few seconds.
Status: builds, installs and launches on the phone. SSH login to `hermes` not
yet verified.

### What was added for xtool (no app code changed)

- `ios/Package.swift`: wraps `ios/Herdcats` as the single library product xtool
  expects, depends on the vendored `ios/ThirdParty/Citadel` as a local package.
  Enables `BareSlashRegexLiterals`, which Xcode turns on by default but SwiftPM
  does not in Swift 5 mode.
- `ios/xtool.yml`: bundle ID, icon, Info.plist path and root bundle resources
  (fonts, `ThirdPartyNotices.txt`, images).
- `ios/xtool-Info.plist`: same keys as upstream `Info.plist` minus Xcode
  `$(...)` variables and the asset catalog launch color.
- `ios/xtool-resources/`: PNGs copied out of `Assets.xcassets`. xtool has no
  `actool`, so asset catalogs are not compiled and images ship as loose files.
- `ios/Package.resolved` is committed to pin the SSH dependency chain.

Upstream's Xcode/XcodeGen setup (`ios/project.yml`) is left untouched.

### Known build caveats

- `Image("HerdrcatLogo")` and agent badges may not resolve from loose PNGs.
  Check on device.
- Launch screen background color came from the asset catalog and is dropped.
- SwiftPM resolves `swift-nio-ssh` (Wellz26 fork) to 0.3.7. Upstream's Xcode
  lockfile says 0.4.0, which is outside Citadel's declared `0.3.4..<0.4.0`
  range, so 0.3.7 is the consistent choice. Keep in mind if SSH behaves
  differently from the TestFlight build.
- Many Swift 6 concurrency warnings. Harmless for now.
- Debug builds include `#if DEBUG` screenshot fixtures.

## Security review of upstream (commit 19e29da)

No exfiltration or suspicious behavior found.

- **Network:** the app makes no HTTP calls from the phone. No `URLSession`,
  analytics, crash reporting or third party SDKs. Only outbound traffic is SSH.
- **Host keys:** unknown keys fail the handshake and require explicit
  fingerprint approval. Changed keys block connection, including auto
  reconnect. Stored in Keychain.
- **Secrets:** Keychain with `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`
  (no iCloud sync, not in backups). Auth is password or pasted OpenSSH ed25519
  key. No key generation.
- **Remote commands:** everything runs as `sh -c` over SSH exec. User input and
  pane ids go through `shellSafeArgument` (printf octal escaping). No injection
  path found.
- **Vendored Citadel 0.12.1:** diffed against upstream `ae8562f`. Only small,
  benign changes in `OpenSSHKey.swift` (bcrypt rounds, padding) and
  `TTY.swift` (close handling). The TTY change is not listed in
  `HERDCATS-PATCH.txt`.
- **Dictation:** on-device when supported, otherwise Apple servers.

### Herdcats Bridge (optional, not installed)

Rust binary in `bridge/`, 19 crates, no network crates. Talks only to the herdr
Unix socket. Installs `PermissionRequest` hooks into Claude/Codex configs and
can disable the Codex daemon. Only used for Claude/Codex, so irrelevant for pi.

Trust weakness: the in-app installer pins `install.sh` by SHA256, but the
binary is only checked against `SHA256SUMS` from the same mutable GitHub
release. If ever needed, build it ourselves with `cargo build` instead.

### Inherent risk

The app can run any command as the SSH user, and a phone can approve agent
actions. Mitigations to apply on `hermes`:

- Dedicated key with `from="100.64.0.0/10",no-port-forwarding,no-agent-forwarding,no-X11-forwarding`
  in `authorized_keys` (only usable over Tailscale, no tunneling into the LAN).
- Optionally a Tailscale ACL allowing only the phone to reach `hermes:22`.

## Architecture notes

Herdcats is a herdr remote control, **not a terminal emulator**. Pane content
is polled with `herdr pane read` (50 ms tick in Live mode), rendered by an own
ANSI parser (`Support/ANSIText.swift`), and input is sent through herdr. Good
for pi (text in, text out), poor for full screen TUIs like vim or htop.

Key files:

- `Networking/HerdrConnection*.swift`: SSH connection, host key validation,
  remote command wrapper, pane input/read
- `Networking/PanePersistentShell.swift`: persistent shell channels for low
  latency input/output
- `Support/KeychainStore.swift`: credentials and `SSHHostKeyStore`
- `Networking/OpenSSHKeyParser.swift`: ed25519 key parsing
- `Features/SpaceDetailView.swift` (3000 lines), `Features/SpacesScreen.swift`,
  `Features/AgentTimeline/` (Claude/Codex only)

## Roadmap

1. **Verify SSH to `hermes`** with the current build.
2. **P-256 key in Secure Enclave:** generate on device, non-exportable, gated
   by Face ID. Show the public key for `authorized_keys`. Check that Citadel's
   P-256 auth can sign with a Secure Enclave key (it may need a custom
   `NIOSSHPrivateKey` path).
3. **Cleanup (later):** remove Claude/Codex/Agy/Cursor/quota, Timeline,
   approvals, questions, bridge client, `web/`, `compatibility/`,
   `ios/review-fly/`. Split the huge views. Fix concurrency warnings.
4. **Real terminal tab:** SSH PTY plus SwiftTerm. Lets you run anything,
   including `herdr` itself as a full fallback. libghostty is a possible later
   experiment but hard to build for iOS from Linux.
5. **Pi native view (idea):** a pi extension streaming structured events, so
   the app renders the conversation natively instead of scraping the screen.
