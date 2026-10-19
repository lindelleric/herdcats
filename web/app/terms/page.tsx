import { SitePage, sitePageStyles as styles } from '@/components/SitePage';

export const metadata = {
  title: 'Herdcats — Terms of Service',
  description: 'Terms for using Herdcats, the open source iPhone client for Herdr.',
};

export default function TermsPage() {
  return (
    <SitePage current="terms" title="Terms of Service">
      <section className={styles.section}>
        <p>
          <strong>Last updated:</strong> 19 October 2026
        </p>
        <p>
          These Terms of Service (&ldquo;Terms&rdquo;) govern your access to and use of the Herdcats
          iPhone application, related websites (including herdcats.dev), and any associated
          documentation or demos (together, the &ldquo;Service&rdquo;).
        </p>
        <p>
          The Service is provided by the publisher of Herdcats (&ldquo;we,&rdquo; &ldquo;us,&rdquo; or
          &ldquo;Publisher&rdquo;). For legal notices, contact{' '}
          <a href="mailto:hello@enchantinglabs.com">hello@enchantinglabs.com</a>.
        </p>
        <p>
          By downloading, installing, accessing, or using the Service, you agree to these Terms.
          If you do not agree, do not use the Service.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>1. What Herdcats Is</h2>
        <p>
          Herdcats is a native iPhone client that connects to machines you control over SSH
          (Tailscale recommended) and presents Herdr spaces, tabs, panes, agents, and related tooling.
          The Service does not host your code, agents, or terminal sessions. Communication is intended
          to be direct between your device and your machine; we do not operate a relay for your SSH traffic.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>2. Eligibility</h2>
        <p>
          You must be able to form a binding contract in your jurisdiction and must comply with
          applicable law (including export and sanctions rules) when using the Service. If you use the
          Service on behalf of an organization, you represent that you have authority to bind that
          organization to these Terms.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>3. Account, Credentials &amp; Your Machine</h2>
        <p>
          You supply your own SSH credentials and connect to hosts you are authorized to access.
          You are solely responsible for:
        </p>
        <ul className={styles.list}>
          <li>The security of passwords, private keys, devices, and Keychain-stored secrets</li>
          <li>Host key verification and decisions to trust or reject fingerprints</li>
          <li>Network exposure of SSH (including keeping SSH off the public internet where appropriate)</li>
          <li>Permissions, sandboxing, and security boundaries on any remote environment you connect to</li>
          <li>All commands, inputs, and agent instructions sent through the Service</li>
        </ul>
        <p>
          Authentication supports passwords and OpenSSH ed25519 private keys, including encrypted keys
          using AES-128-CTR or AES-256-CTR with bcrypt at 1–256 rounds. Other key formats and encryption
          settings are unsupported.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>4. Remote Execution &amp; High-Risk Use</h2>
        <p>
          Commands and agent activity invoked via Herdcats execute on your remote environment,
          not on our servers. Agents may modify files, run shell commands, spend provider quotas,
          or take other irreversible actions. You accept all risk arising from remote execution,
          misconfiguration, compromised credentials, or agent behavior.
        </p>
        <p>
          The Service can, at your direction, download and install optional Herdcats Bridge
          software (<code>herdcats-bridge</code>) on your remote host from our GitHub releases. Downloads are verified by
          checksum but are not independently code-signed; you are responsible for reviewing and
          authorizing installations on machines you control.
        </p>
        <p>
          Only when you tap &ldquo;Turn off Codex&rsquo;s shared daemon&rdquo; and confirm,
          Herdcats Bridge changes <code>daemon_auto_start</code> to <code>false</code> in
          <code>~/.codex/config.toml</code> (or <code>$CODEX_HOME/config.toml</code> if you set <code>CODEX_HOME</code>) and stops Codex&rsquo;s shared background server
          if running. It keeps a config backup and leaves other settings unchanged. Codex
          sessions using the server disconnect and must be restarted with <code>codex</code>
          or <code>codex resume</code>.
        </p>
        <p>
          The Service is a developer tool. It is not designed for use in safety-critical, medical,
          emergency, or other high-risk environments where failure could cause death, personal injury,
          or severe physical or environmental damage.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>5. Acceptable Use</h2>
        <p>You agree not to:</p>
        <ul className={styles.list}>
          <li>Use the Service to access systems you are not authorized to access</li>
          <li>Violate applicable law, including computer misuse, export, privacy, or IP laws</li>
          <li>Attempt to probe, disrupt, or bypass security of the Service or others&rsquo; systems via the Service</li>
          <li>Misrepresent the Service as providing a hosted backend, relay, or managed agent platform that we operate</li>
          <li>Reverse engineer the Service except to the extent such restriction is prohibited by law or permitted by the open source license</li>
        </ul>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>6. Purchases, Trial &amp; Apple</h2>
        <p>
          Viewing and connecting to your own machines may be available without purchase. Certain
          features (such as sending input or making remote changes) may require a time-limited trial
          and/or a one-time in-app purchase. Exact pricing, trial length, and feature gates are shown
          in the App Store listing and in the app at the time of purchase.
        </p>
        <p>
          If you buy through the Apple App Store, payment is processed by Apple. Apple&rsquo;s terms,
          billing practices, and refund policies apply to those transactions. We do not store your
          full payment card details. AI provider subscriptions, API credits, Tailscale, or other
          third-party services are separate and not included in any Herdcats purchase.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>7. Open Source License</h2>
        <p>
          Source code for Herdcats may be made available under the GNU General Public License
          version 3, with an additional permission for Apple App Store and Google Play distribution,
          as stated in the project&rsquo;s LICENSE file. These Terms govern your use of the distributed
          Service and do not reduce rights you have under that license for the code itself. If there
          is a conflict between these Terms and the LICENSE regarding rights to copy or modify the
          source code, the LICENSE controls for those rights.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>8. Third-Party Services</h2>
        <p>
          The Service may interoperate with third-party software and services you choose (for example
          Herdr, SSH hosts, Tailscale, AI agent CLIs, quota tooling, and GitHub, which serves
          optional Herdcats Bridge downloads). Those services are governed
          by their own terms and privacy policies. We do not control them and are not responsible for
          their availability, security, billing, or conduct.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>9. Intellectual Property</h2>
        <p>
          Subject to the open source license and any third-party rights, we and our licensors retain
          all rights in the Service&rsquo;s branding, design, and non-open-source materials. You retain
          all rights in your content, credentials, code, and data on your machines. You grant us no
          license to that material by using a direct SSH connection that never sends it to us.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>10. Privacy</h2>
        <p>
          Our practices are described in the{' '}
          <a href="/privacy">Privacy Policy</a>. Please read it. In short, the app is designed so that
          SSH traffic goes to your machine, and credentials you choose to store remain in the on-device
          Keychain.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>11. Disclaimer of Warranties</h2>
        <p>
          THE SERVICE IS PROVIDED &ldquo;AS IS&rdquo; AND &ldquo;AS AVAILABLE,&rdquo; WITHOUT WARRANTIES OF ANY KIND,
          WHETHER EXPRESS, IMPLIED, OR STATUTORY, INCLUDING MERCHANTABILITY, FITNESS FOR A PARTICULAR
          PURPOSE, TITLE, AND NON-INFRINGEMENT. WE DO NOT WARRANT THAT THE SERVICE WILL BE
          UNINTERRUPTED, SECURE, OR ERROR-FREE, OR THAT REMOTE COMMANDS OR AGENTS WILL PRODUCE ANY
          PARTICULAR RESULT.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>12. Limitation of Liability</h2>
        <p>
          TO THE MAXIMUM EXTENT PERMITTED BY LAW, WE AND OUR CONTRIBUTORS WILL NOT BE LIABLE FOR ANY
          INDIRECT, INCIDENTAL, SPECIAL, CONSEQUENTIAL, EXEMPLARY, OR PUNITIVE DAMAGES, OR FOR ANY LOSS
          OF PROFITS, DATA, GOODWILL, BUSINESS INTERRUPTION, COMPUTER DAMAGE, OR COST OF SUBSTITUTE
          SERVICES, ARISING FROM OR RELATED TO THE SERVICE OR THESE TERMS — INCLUDING DAMAGES FROM
          REMOTE COMMANDS, AGENT ACTIONS, CREDENTIAL LOSS, OR UNAUTHORIZED ACCESS TO YOUR MACHINES —
          EVEN IF ADVISED OF THE POSSIBILITY.
        </p>
        <p>
          TO THE MAXIMUM EXTENT PERMITTED BY LAW, OUR TOTAL LIABILITY FOR ANY CLAIM ARISING OUT OF OR
          RELATING TO THE SERVICE OR THESE TERMS WILL NOT EXCEED THE GREATER OF (A) THE AMOUNT YOU
          PAID US FOR THE SERVICE IN THE TWELVE MONTHS BEFORE THE CLAIM OR (B) FIFTY US DOLLARS (US$50).
        </p>
        <p>
          Some jurisdictions do not allow certain limitations; in those cases, our liability is limited
          to the fullest extent permitted.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>13. Indemnity</h2>
        <p>
          You will defend, indemnify, and hold harmless the Publisher and its contributors from and
          against claims, damages, losses, and expenses (including reasonable legal fees) arising out
          of your use of the Service, your remote environments, your credentials, your agents&rsquo;
          actions, or your violation of these Terms or applicable law.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>14. Suspension &amp; Termination</h2>
        <p>
          You may stop using the Service at any time (including by deleting the app). We may suspend
          or stop offering the Service, or deny access where we reasonably believe you have violated
          these Terms or that continued use creates legal or security risk. Provisions that by their
          nature should survive (including §§11–13, 15–17) will survive termination.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>15. Changes</h2>
        <p>
          We may update these Terms from time to time. We will post the updated Terms with a revised
          &ldquo;Last updated&rdquo; date. Material changes may also be noted in the app or on the website.
          Continued use after the effective date constitutes acceptance of the updated Terms.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>16. Governing Law &amp; Disputes</h2>
        <p>
          These Terms are governed by the laws of Singapore, without regard to conflict-of-law rules.
          The courts of Singapore will have exclusive jurisdiction, except that we may seek
          injunctive relief in any jurisdiction, and nothing limits mandatory consumer protections
          that apply to you.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>17. General</h2>
        <p>
          These Terms are the entire agreement between you and us regarding the Service, and supersede
          prior terms on the same subject. If a provision is unenforceable, the remainder stays in
          effect. Failure to enforce a provision is not a waiver. You may not assign these Terms
          without our consent; we may assign them in connection with a reorganization or sale.
          Sections titles are for convenience only.
        </p>
        <p>
          Questions:{' '}
          <a href="mailto:hello@enchantinglabs.com">hello@enchantinglabs.com</a>.
        </p>
      </section>

    </SitePage>
  );
}
