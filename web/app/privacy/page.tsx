import { SitePage, sitePageStyles as styles } from '@/components/SitePage';

export const metadata = {
  title: 'Herdcats — Privacy Policy',
  description:
    'How Herdcats handles data: no telemetry, direct SSH to your machine, on-device Keychain, and optional Apple Speech dictation.',
};

export default function PrivacyPage() {
  return (
    <SitePage current="privacy" title="Privacy Policy">
      <section className={styles.section}>
        <p>
          <strong>Last updated:</strong> 19 October 2026
        </p>
        <p>
          This Privacy Policy explains how the publisher of Herdcats (&ldquo;we,&rdquo; &ldquo;us,&rdquo; or
          &ldquo;Publisher&rdquo;) handles information in connection with the Herdcats iPhone app,
          related websites (including herdcats.dev), and associated documentation or demos
          (together, the &ldquo;Service&rdquo;).
        </p>
        <p>
          Contact:{' '}
          <a href="mailto:hello@enchantinglabs.com">hello@enchantinglabs.com</a>.
        </p>
        <p>
          Herdcats is designed so that your SSH sessions, source code, terminal output, and
          agent activity stay between your iPhone and machines you control. We do not operate a
          relay, backend, or cloud daemon for those sessions.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>1. Summary</h2>
        <ul className={styles.list}>
          <li>No analytics SDKs, advertising trackers, or usage telemetry from us</li>
          <li>SSH traffic goes to your host (or a network path you choose, such as Tailscale)</li>
          <li>Credentials and host-key pins you save live in the on-device iOS Keychain</li>
          <li>Optional drafts and command history can be stored locally on your device</li>
          <li>Optional dictation uses Apple&rsquo;s Speech frameworks (and may involve Apple)</li>
          <li>Optional photo attachments are sent to your remote machine over SSH</li>
          <li>Optional Herdcats Bridge is downloaded from GitHub releases and runs on your own machine, over your SSH connection</li>
          <li>App Store purchases are processed by Apple</li>
        </ul>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>2. Information We Do Not Collect</h2>
        <p>
          We do not run analytics, crash-reporting, advertising, or telemetry SDKs in the app.
          We do not receive your SSH passwords, private keys, pane output, source code, worktrees,
          or agent transcripts on our servers, because those servers are not in the path of your
          SSH connection.
        </p>
        <p>
          If you contact us by email, we receive whatever you choose to include in that message
          so we can respond.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>3. Information on Your Device</h2>
        <p>Depending on how you use the app, the following may be stored locally on your iPhone:</p>
        <ul className={styles.list}>
          <li>
            <strong>Credentials.</strong> Passwords, OpenSSH ed25519 private keys, and optional key passphrases you choose to remember, stored in the iOS Keychain with
            device-only protection (WhenUnlockedThisDeviceOnly). Private keys and key passphrases remain on your device. Passwords are sent only to hosts you connect to through encrypted SSH authentication.
          </li>
          <li>
            <strong>Connection profile.</strong> Recent connection
            details (such as host, port, and username) and approved SSH host-key fingerprints,
            stored in Keychain and checked on later connects, including automatic reconnect.
          </li>
          <li>
            <strong>App preferences.</strong> Settings such as
            onboarding state and Pane Data &amp; Privacy choices (for example whether to preserve
            drafts and command history), typically via ordinary app preferences / UserDefaults.
          </li>
          <li>
            <strong>Drafts and history.</strong> When Preserve Drafts
            and Command History is enabled (default), unsent drafts and sent-message history keyed
            by connection and pane may be stored locally. Turning the setting off or clearing saved
            data erases that local material; it does not clear remote panes.
          </li>
          <li>
            <strong>In-session UI state.</strong> Temporary in-memory
            state needed to run the app (for example connection phase, queues, or debug latency
            samples in non-production builds). Debug diagnostics are not present in production
            builds and do not record keystrokes, output, or credentials.
          </li>
        </ul>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>4. SSH, Your Machine &amp; Agents</h2>
        <p>
          When you connect, the app opens an encrypted SSH session (using the Citadel library) to
          a host you specify. Pane output, diffs, agent activity, and commands you send travel on
          that session. We do not host or store that content.
        </p>
        <p>
          If you use a VPN or mesh network you choose (for example Tailscale), that provider may
          carry encrypted traffic under its own terms. Keep SSH off the public internet where
          appropriate; Tailscale is the recommended way to reach your machine.
        </p>
        <p>
          Photo attachments you select are processed on device (downscaled to at most 2048 pixels and
          re-encoded, which drops location metadata) and uploaded over SSH to a cache path on your remote machine
          (for example under <code>~/.cache/herdrcat/attachments</code>). The app does not
          automatically delete those remote files.
        </p>
        <p>
          Optional Herdcats Bridge. If you enable the native Timeline, the app can download
          Herdcats Bridge (<code>herdcats-bridge</code>) from our GitHub releases and install it on the direct
          SSH host (at <code>~/.local/bin/herdcats-bridge</code>) after you tap Install. GitHub serves
          that download and may log the request under its own privacy policy. Herdcats Bridge reads the
          agent session transcript on that machine (Claude and Codex) and sends it to your phone
          over the same SSH connection. It runs only while Timeline is open; there is no background
          service. Downloads are checksum-verified but not code-signed or notarized. Delete the
          file to uninstall it.
        </p>
        <p>
          If you tap &ldquo;Turn off Codex&rsquo;s shared daemon&rdquo; and confirm, Herdcats Bridge
          sets <code>daemon_auto_start = false</code> in <code>~/.codex/config.toml</code> (or <code>$CODEX_HOME/config.toml</code> if you set <code>CODEX_HOME</code>),
          keeping a backup and leaving other settings unchanged. It also stops Codex&rsquo;s
          shared background server if running, disconnecting sessions that use it. Restart
          those sessions with <code>codex</code> or <code>codex resume</code>.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>5. Microphone &amp; Dictation</h2>
        <p>
          Optional Voice / dictation uses the microphone and Apple&rsquo;s Speech frameworks. The app
          requests microphone and speech-recognition permission when you use that feature.
          Recognized text can be placed into an editable Compose draft for your review; it is not
          auto-sent.
        </p>
        <p>
          Speech recognition is provided by Apple. Depending on your device settings and Apple&rsquo;s
          systems, audio or transcripts may be processed by Apple under{' '}
          <a href="https://www.apple.com/privacy/" target="_blank" rel="noopener noreferrer">
            Apple&rsquo;s privacy policy
          </a>
          . We do not operate a separate speech cloud for Herdcats.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>6. Purchases</h2>
        <p>
          If you buy a one-time unlock or start a trial through the Apple App Store, payment and
          subscription/trial state are handled by Apple. We do not receive or store your full
          payment card details. Apple&rsquo;s privacy policy applies to those transactions.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>7. Website</h2>
        <p>
          Our website may be hosted on infrastructure operated by third parties (for example
          Cloudflare Pages). Standard web server or CDN logs (such as IP address, user agent, and
          requested URL) may be collected by those hosts as part of delivering the site. The
          marketing site is separate from your SSH sessions in the app.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>8. How We Use Information</h2>
        <p>Where information reaches us at all (primarily email support), we use it to:</p>
        <ul className={styles.list}>
          <li>Respond to support or legal requests</li>
          <li>Improve documentation and the Service based on what you tell us</li>
          <li>Comply with law and enforce our Terms</li>
        </ul>
        <p>
          On-device data is used only to provide the features you enable (connect, remember
          hosts, preserve drafts, dictate, attach photos, and restore purchases via Apple).
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>9. Sharing</h2>
        <p>
          We do not sell your personal information. We do not share SSH session content with
          third parties because we do not receive it. Categories of third parties that may process
          data in connection with the Service include:
        </p>
        <ul className={styles.list}>
          <li>Apple (App Store / StoreKit, and Speech if you use dictation)</li>
          <li>Website hosting / CDN providers for herdcats.dev</li>
          <li>GitHub (serves the optional Herdcats Bridge download, if you install it)</li>
          <li>Networks and hosts you configure (your SSH server, Tailscale, Herdr, AI agent tools)</li>
        </ul>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>10. Retention &amp; Deletion</h2>
        <p>
          Local credentials, host-key pins, drafts, and preferences remain on your device until you
          delete them in the app, clear app data, or uninstall the app (subject to iOS behavior).
          Email you send us is retained as long as needed to handle your request and meet legal
          obligations. Remote attachments and pane data on your machine are under your control.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>11. Security</h2>
        <p>
          We design the app to fail closed on host-key changes, keep remembered secrets in the
          Keychain, and avoid logging credentials. No method of transmission or storage is
          perfectly secure. You remain responsible for device lock, key hygiene, and the security
          of machines you connect to.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>12. Children</h2>
        <p>
          The Service is a developer tool and is not directed at children under 13 (or the
          equivalent minimum age in your jurisdiction). We do not knowingly collect personal
          information from children. If you believe a child has provided us information by email,
          contact us and we will delete it.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>13. International Users</h2>
        <p>
          We are oriented around Singapore for the Publisher&rsquo;s operations and for the Terms of
          Service. If you use the Service from elsewhere, you are responsible for compliance with
          local law. Because SSH content stays on your path to your machine, cross-border transfer
          of that content is determined by where you and your hosts are, not by a Publisher-operated
          processing pipeline.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>14. Your Choices &amp; Rights</h2>
        <p>You can:</p>
        <ul className={styles.list}>
          <li>Decline to remember credentials, or remove saved connections and secrets in the app</li>
          <li>Turn off or clear local drafts and command history in Settings → Pane Data &amp; Privacy</li>
          <li>Deny microphone or speech permission in iOS Settings (dictation will not work)</li>
          <li>Avoid selecting photos if you do not want attachments on your remote host</li>
          <li>Uninstall the app</li>
          <li>Email us to access, correct, or delete personal information we hold (typically email correspondence)</li>
        </ul>
        <p>
          Depending on where you live, you may have additional rights under applicable privacy law.
          Contact us to exercise them. You may also have rights regarding data held by Apple or by
          operators of machines and networks you use; those requests go to those parties.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>15. Changes</h2>
        <p>
          We may update this Privacy Policy from time to time. We will post the updated policy with
          a revised &ldquo;Last updated&rdquo; date. Material changes may also be noted in the app or on
          the website. Continued use after the effective date means you accept the updated policy.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>16. Related Terms</h2>
        <p>
          Use of the Service is also subject to our{' '}
          <a href="/terms">Terms of Service</a>.
        </p>
      </section>
    </SitePage>
  );
}
