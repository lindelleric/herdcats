'use client';

import React, { useEffect, useRef, useState } from 'react';
import { TriangleAlert } from 'lucide-react';
import { SiteHeader } from '@/components/SiteHeader';
import { SiteFooter } from '@/components/SitePage';

const supportSections = [
  {
    "id": "faq",
    "label": "Getting started",
    "depth": 0
  },
  {
    "id": "faq-what-is-herdcats",
    "label": "About Herdcats",
    "depth": 1
  },
  {
    "id": "faq-what-is-herdr-and-how-is-it-different-from-herdcats",
    "label": "Herdr and Herdcats",
    "depth": 1
  },
  {
    "id": "faq-what-do-i-need-before-i-connect",
    "label": "Requirements",
    "depth": 1
  },
  {
    "id": "faq-can-i-use-herdcats-if-my-computer-doesn-t-have-herdr-installed",
    "label": "Herdr required",
    "depth": 1
  },
  {
    "id": "faq-do-i-need-tailscale",
    "label": "Tailscale",
    "depth": 1
  },
  {
    "id": "faq-do-i-need-any-other-software-on-my-computer",
    "label": "Required software",
    "depth": 1
  },
  {
    "id": "privacy",
    "label": "Signing in and privacy",
    "depth": 0
  },
  {
    "id": "privacy-can-i-sign-in-with-a-password-or-ssh-key",
    "label": "Passwords and SSH keys",
    "depth": 1
  },
  {
    "id": "privacy-do-i-have-to-enter-my-key-s-passphrase-every-time",
    "label": "Remembering passphrases",
    "depth": 1
  },
  {
    "id": "privacy-what-is-the-fingerprint-the-app-asks-me-to-approve",
    "label": "Server fingerprints",
    "depth": 1
  },
  {
    "id": "privacy-why-is-a-changed-host-key-blocking-my-connection",
    "label": "Changed host keys",
    "depth": 1
  },
  {
    "id": "privacy-where-are-my-saved-sign-in-details-stored",
    "label": "Saved credentials",
    "depth": 1
  },
  {
    "id": "privacy-is-voice-dictation-always-processed-on-my-iphone",
    "label": "Dictation privacy",
    "depth": 1
  },
  {
    "id": "using-app",
    "label": "Using the app",
    "depth": 0
  },
  {
    "id": "using-app-what-are-spaces-agents-tabs-and-panes",
    "label": "Spaces and panes",
    "depth": 1
  },
  {
    "id": "using-app-when-should-i-use-live-compose-or-voice",
    "label": "Live, Compose, Voice",
    "depth": 1
  },
  {
    "id": "using-app-what-is-the-timeline-helper-and-why-does-timeline-need-it",
    "label": "Herdcats Bridge",
    "depth": 1
  },
  {
    "id": "using-app-codex-shared-background-server",
    "label": "Codex background server",
    "depth": 1
  },
  {
    "id": "using-app-does-herdcats-run-agents-on-my-iphone",
    "label": "Where agents run",
    "depth": 1
  },
  {
    "id": "using-app-can-i-send-photos",
    "label": "Photos",
    "depth": 1
  },
  {
    "id": "using-app-what-happens-when-i-switch-apps-or-lock-my-phone",
    "label": "Background refresh",
    "depth": 1
  },
  {
    "id": "using-app-can-i-choose-a-different-herdr-session",
    "label": "Herdr sessions",
    "depth": 1
  },
  {
    "id": "using-app-can-i-switch-between-computers-without-signing-in-to-each-one-on-my-phone",
    "label": "Switching machines",
    "depth": 1
  },
  {
    "id": "using-app-how-do-i-add-another-machine",
    "label": "Adding machines",
    "depth": 1
  },
  {
    "id": "connect",
    "label": "How to connect",
    "depth": 0
  },
  {
    "id": "macos-setup",
    "label": "SSH on macOS",
    "depth": 0
  },
  {
    "id": "macos-setup-enable-remote-login",
    "label": "Remote Login",
    "depth": 1
  },
  {
    "id": "macos-setup-generate-an-ed25519-key-authorize-it",
    "label": "Create an SSH key",
    "depth": 1
  },
  {
    "id": "macos-setup-transfer-the-private-key-to-herdcats",
    "label": "Import your key",
    "depth": 1
  },
  {
    "id": "macos-setup-find-host-username",
    "label": "Host and username",
    "depth": 1
  },
  {
    "id": "linux-setup",
    "label": "SSH on Linux",
    "depth": 0
  },
  {
    "id": "linux-setup-ensure-openssh-server-is-installed-running",
    "label": "Install OpenSSH",
    "depth": 1
  },
  {
    "id": "linux-setup-generate-authorize-key",
    "label": "Authorize your key",
    "depth": 1
  },
  {
    "id": "linux-setup-check-firewall-herdr-binary-location",
    "label": "Firewall and Herdr",
    "depth": 1
  },
  {
    "id": "windows-setup",
    "label": "SSH on Windows / WSL",
    "depth": 0
  },
  {
    "id": "windows-setup-option-a-wsl-2-recommended",
    "label": "WSL 2",
    "depth": 1
  },
  {
    "id": "windows-setup-option-b-native-windows-openssh-server",
    "label": "Windows OpenSSH",
    "depth": 1
  },
  {
    "id": "ssh-keys",
    "label": "SSH keys",
    "depth": 0
  },
  {
    "id": "ssh-keys-why-ed25519",
    "label": "Ed25519 keys",
    "depth": 1
  },
  {
    "id": "ssh-keys-passphrase-notice",
    "label": "Passphrases",
    "depth": 1
  },
  {
    "id": "ssh-keys-on-device-keychain",
    "label": "Keychain storage",
    "depth": 1
  },
  {
    "id": "ssh-keys-shoulder-surfing-protection",
    "label": "Key masking",
    "depth": 1
  },
  {
    "id": "troubleshooting",
    "label": "Troubleshooting",
    "depth": 0
  },
  {
    "id": "troubleshooting-authentication-failed-check-username-and-password-key",
    "label": "Authentication failed",
    "depth": 1
  },
  {
    "id": "troubleshooting-host-key-verification-failed-changed",
    "label": "Host key errors",
    "depth": 1
  },
  {
    "id": "troubleshooting-connection-timeout-host-unreachable",
    "label": "Connection timeout",
    "depth": 1
  },
  {
    "id": "troubleshooting-herdr-command-not-found",
    "label": "Herdr not found",
    "depth": 1
  },
  {
    "id": "troubleshooting-timeline-helper-missing-or-install-failed",
    "label": "Herdcats Bridge",
    "depth": 1
  },
  {
    "id": "troubleshooting-codex-session-not-identified",
    "label": "Codex session not identified",
    "depth": 1
  }
];

export default function SupportPage() {
  const scrollRoot = useRef<HTMLDivElement>(null);
  const [activeSection, setActiveSection] = useState('faq');

  useEffect(() => {
    const root = scrollRoot.current;
    if (!root) return;
    let frame = 0;
    const updateSection = () => {
      frame = 0;
      const threshold = root.getBoundingClientRect().top + (window.innerWidth <= 800 ? 100 : 48);
      let current = supportSections[0].id;
      for (const section of supportSections) {
        const element = document.getElementById(section.id);
        if (element && element.getBoundingClientRect().top <= threshold) current = section.id;
      }
      if (root.scrollTop > 0 && root.scrollHeight - root.scrollTop - root.clientHeight <= 4) {
        current = supportSections[supportSections.length - 1].id;
      }
      setActiveSection(current);
    };
    const scheduleUpdate = () => {
      if (!frame) frame = requestAnimationFrame(updateSection);
    };
    const observer = new ResizeObserver(scheduleUpdate);
    const content = root.querySelector('.support-content');
    if (content) observer.observe(content);
    root.addEventListener('scroll', scheduleUpdate, { passive: true });
    window.addEventListener('resize', scheduleUpdate);
    updateSection();
    return () => {
      root.removeEventListener('scroll', scheduleUpdate);
      window.removeEventListener('resize', scheduleUpdate);
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, []);
  useEffect(() => {
    const keepActiveLinkVisible = () => {
      const sidebar = scrollRoot.current?.querySelector<HTMLElement>('.support-sidebar');
      const link = sidebar?.querySelector<HTMLElement>('[aria-current="location"]');
      if (!sidebar || !link) return;
      const mobile = window.matchMedia('(max-width: 800px)').matches;
      const container = mobile ? sidebar.querySelector<HTMLElement>('nav') : sidebar;
      if (!container) return;
      const viewport = container.getBoundingClientRect();
      const target = link.getBoundingClientRect();
      // Scroll only the navigation container, leaving the article in place.
      if (mobile) {
        const left = target.left < viewport.left ? target.left - viewport.left
          : target.right > viewport.right ? target.right - viewport.right : 0;
        if (left) container.scrollBy({ left, behavior: 'instant' });
      } else {
        const top = target.top < viewport.top ? target.top - viewport.top
          : target.bottom > viewport.bottom ? target.bottom - viewport.bottom : 0;
        if (top) container.scrollBy({ top, behavior: 'instant' });
      }
    };
    keepActiveLinkVisible();
    window.addEventListener('resize', keepActiveLinkVisible);
    return () => window.removeEventListener('resize', keepActiveLinkVisible);
  }, [activeSection]);

  const [copiedIndex, setCopiedIndex] = useState<string | null>(null);

  const copyToClipboard = (text: string, id: string) => {
    navigator.clipboard.writeText(text);
    setCopiedIndex(id);
    setTimeout(() => setCopiedIndex(null), 2000);
  };

  return (
    <div className="support-wrapper" ref={scrollRoot}>
      <SiteHeader current="support" title="Support">
        <p>Find answers, connect your iPhone to Herdr, and switch between your machines.</p>
        <p>Still stuck? Email <a href="mailto:hello@enchantinglabs.com">hello@enchantinglabs.com</a> or open an issue on <a href="https://github.com/zhiyao/herdcats/issues" target="_blank" rel="noopener noreferrer">GitHub</a>. Leave out passwords, keys and passphrases.</p>
      </SiteHeader>

      {/* Main Content */}
      <main className="support-main">
        <aside className="support-sidebar">
          <nav aria-label="Support sections">
            <p className="sidebar-title">Support guide</p>
            {supportSections.map((section) => (
              <a key={section.id} href={`#${section.id}`}
                className={`section-link${section.depth ? ' subsection-link' : ''}${activeSection === section.id ? ' active' : ''}`}
                aria-current={activeSection === section.id ? 'location' : undefined}>
                {section.label}
              </a>
            ))}
          </nav>
        </aside>
        <div className="support-content">
        {/* Hero Section */}
        <section className="hero-section">
          <div className="security-banner">
            <div className="security-text">
              <strong>Direct &amp; Secure:</strong> No intermediate relays, proxies, or cloud daemons.
              Credentials stay encrypted in your iPhone’s on-device Keychain.
              <span className="ts-tag">Tailscale Recommended</span>
            </div>
          </div>
        </section>

        <section id="faq" aria-labelledby="faq-title" className="guide-card faq-section">
        <h2 id="faq-title">Frequently asked questions</h2>
        <p className="card-desc">Help with connecting your iPhone and using Herdcats. Some features depend on your app version.</p>
        <h3 className="faq-category">Getting started</h3>
        <div className="faq-list">
        <div className="faq-item">
        <h4 id="faq-what-is-herdcats" className="faq-question">What is Herdcats?</h4>
        <p>Herdcats lets you use Herdr from your iPhone: browse your spaces and agents, read terminal output, and send messages, keystrokes, dictation, and photos. Connect to one Herdr session to switch between that computer and other machines connected through Herdr. It requires iOS 17 or later.</p>
        </div>
        <div className="faq-item">
        <h4 id="faq-what-is-herdr-and-how-is-it-different-from-herdcats" className="faq-question">What is Herdr, and how is it different from Herdcats?</h4>
        <p>Herdr runs on your computer and organizes the spaces, agents, and terminal panes you work with. Herdcats is the iPhone app that connects to it. Your agents and their work stay on the computer.</p>
        </div>
        <div className="faq-item">
        <h4 id="faq-what-do-i-need-before-i-connect" className="faq-question">What do I need before I connect?</h4>
        <p>You need:</p>
        <ul>
        <li>A computer with Herdr installed and its default session running.</li>
        <li>SSH access enabled on that computer. SSH lets the app connect securely.</li>
        <li>A network connection that lets your iPhone reach the computer.</li>
        <li>The computer&#x27;s hostname or IP address, SSH port, account username, and   password or a supported SSH private key.</li>
        </ul>
        <p>Use the computer account that runs Herdr when signing in from Herdcats. If that Herdr session already has other machines configured, you can access them through the same connection from your phone.</p>
        <p>Follow <a href="#connect">How to connect</a> to enable SSH on your computer.</p>
        </div>
        <div className="faq-item">
        <h4 id="faq-can-i-use-herdcats-if-my-computer-doesn-t-have-herdr-installed" className="faq-question">Can I use Herdcats if my computer doesn&#x27;t have Herdr installed?</h4>
        <p>No. Herdcats needs Herdr on the computer to show spaces, agents, and panes. SSH access alone is not enough. If SSH connects but Herdr is missing, the app reports <strong>“herdr was not found on the remote machine.”</strong></p>
        <p>Install Herdr on that computer, start its default session using the account that you connect with, then try again. Herdcats does not install Herdr for you or provide a standalone SSH terminal. If someone else manages the computer, ask them to set up Herdr and SSH access for your account.</p>
        </div>
        <div className="faq-item">
        <h4 id="faq-do-i-need-tailscale" className="faq-question">Do I need Tailscale?</h4>
        <p>Tailscale is recommended for connecting your phone and computer, including when you are away from home. Both devices should be on the same Tailscale network. Another network works too, as long as your phone can reach the computer&#x27;s SSH server.</p>
        </div>
        <div className="faq-item">
        <h4 id="faq-do-i-need-any-other-software-on-my-computer" className="faq-question">Do I need any other software on my computer?</h4>
        <p>Herdr and SSH are required. Terminal, Compose, voice, and photos need nothing else. The optional Timeline view does: Herdcats Bridge (<code>herdcats-bridge</code>) on the computer you connect to directly. A terminal pane is a scrolling log; Herdcats Bridge reads your Claude and Codex sessions&#x27; structured transcripts so Herdcats can show them as readable messages, tool activity, and images instead. It is installed only when you tap Install, runs only while Timeline is open, and everything else works without it. The optional <code>quota-axi</code> tool adds provider quota information.</p>
        </div>
        </div>
        <h3 id="privacy" className="faq-category">Signing in and privacy</h3>
        <div className="faq-list">
        <div className="faq-item">
        <h4 id="privacy-can-i-sign-in-with-a-password-or-ssh-key" className="faq-question">Can I sign in with a password or SSH key?</h4>
        <p>You can use your computer account&#x27;s password or an OpenSSH ed25519 private key. Other private-key formats are unsupported.</p>
        <p>Passphrase-protected ed25519 keys are supported with these settings: AES-128-CTR or AES-256-CTR encryption and bcrypt at 1–256 rounds. If your key is rejected, check your app version and ask your administrator to confirm the key format.</p>
        </div>
        <div className="faq-item">
        <h4 id="privacy-do-i-have-to-enter-my-key-s-passphrase-every-time" className="faq-question">Do I have to enter my key&#x27;s passphrase every time?</h4>
        <p>Remembering the passphrase is optional. If you choose not to save it, you will need to unlock the key again after restarting the app.</p>
        </div>
        <div className="faq-item">
        <h4 id="privacy-what-is-the-fingerprint-the-app-asks-me-to-approve" className="faq-question">What is the fingerprint the app asks me to approve?</h4>
        <p>It identifies the computer&#x27;s SSH server. Checking it helps ensure you are connecting to the intended computer. Compare it with a fingerprint obtained directly from that computer or its administrator before approving it. Herdcats remembers the approved host key for that hostname and port.</p>
        </div>
        <div className="faq-item">
        <h4 id="privacy-why-is-a-changed-host-key-blocking-my-connection" className="faq-question">Why is a changed host key blocking my connection?</h4>
        <p>The computer is presenting a different identity from the one you approved. This can happen after a server is rebuilt, but you should verify the reason with its administrator before restoring access. Herdcats blocks the connection, including automatic reconnects. When the changed-key review appears, compare the new fingerprint with one obtained directly from your computer or its administrator through a trusted channel. Confirm that you verified it, then choose “Replace Approved Key and Connect”. Canceling keeps the previously approved key.</p>
        </div>
        <div className="faq-item">
        <h4 id="privacy-where-are-my-saved-sign-in-details-stored" className="faq-question">Where are my saved sign-in details stored?</h4>
        <p>Remembered credentials and approved host keys are stored in your iPhone&#x27;s Keychain. Keep passwords, private keys, and passphrases out of screenshots and support reports.</p>
        </div>
        <div className="faq-item">
        <h4 id="privacy-is-voice-dictation-always-processed-on-my-iphone" className="faq-question">Is voice dictation always processed on my iPhone?</h4>
        <p>Herdcats uses Apple&#x27;s speech recognition and requests on-device processing when available. Availability depends on your device and language, so local processing is not guaranteed. Dictation requires microphone and speech recognition permissions. Review the text before sending it.</p>
        </div>
        </div>
        <h3 id="using-app" className="faq-category">Using the app</h3>
        <div className="faq-list">
        <div className="faq-item">
        <h4 id="using-app-what-are-spaces-agents-tabs-and-panes" className="faq-question">What are spaces, agents, tabs, and panes?</h4>
        <p>These are the items in your Herdr session. Spaces organize your work; tabs contain terminal panes; agents run in those panes. Herdcats lets you navigate that work from your phone.</p>
        </div>
        <div className="faq-item">
        <h4 id="using-app-when-should-i-use-live-compose-or-voice" className="faq-question">When should I use Live, Compose, or Voice?</h4>
        <ul>
        <li><strong>Live:</strong> interact with the terminal, including special keys in the toolbar.</li>
        <li><strong>Compose:</strong> write and review a message before sending it.</li>
        <li><strong>Voice:</strong> dictate a message, review the text, then send it.</li>
        </ul>
        </div>
        <div className="faq-item">
        <h4 id="using-app-what-is-the-timeline-helper-and-why-does-timeline-need-it" className="faq-question">What is Herdcats Bridge, and why does Timeline need it?</h4>
        <p>Timeline turns an agent session into a phone-friendly conversation: readable messages, tool activity, and images. A terminal pane can&#x27;t provide that — its output is a scrolling log. Herdcats Bridge (<code>herdcats-bridge</code>) is a small program on your SSH computer that reads the session&#x27;s structured transcript and sends it to your phone over the same SSH connection, only while Timeline is open.</p>
        <p>It is optional. Terminal, Compose, voice, and photos never need it, and Timeline asks before installing anything: no sudo, no background service, no agent hooks. Timeline currently supports Claude and Codex; other agents keep using Terminal and Compose. Delete <code>~/.local/bin/herdcats-bridge</code> to remove it.</p>
        </div>
        <div className="faq-item">
        <h4 id="using-app-codex-shared-background-server" className="faq-question">Why does Codex Timeline ask me to turn off Codex&#x27;s shared background server?</h4>
        <p>Since Codex 0.157, Codex runs sessions inside one shared background server. That server only remembers the terminal that started it, so Herdr can&#x27;t tell which pane a Codex conversation belongs to. Herdcats never guesses: showing the wrong conversation is worse than showing none, so Timeline says the session isn&#x27;t identified.</p>
        <p>There are two ways to fix it:</p>
        <ol>
        <li>Start Codex with <code>codex --no-daemon</code> in the panes you want in Timeline.</li>
        <li>Turn the shared server off for all sessions: in a Codex pane&#x27;s Timeline, tap <strong>Turn off Codex&#x27;s shared daemon</strong>. Herdcats Bridge sets <code>daemon_auto_start = false</code> in <code>~/.codex/config.toml</code> (or <code>$CODEX_HOME/config.toml</code> if you set <code>CODEX_HOME</code>), keeps a backup, and changes nothing else in the file. If the server is already running, it stops it after you confirm. Codex sessions using it disconnect; start them again with <code>codex</code>, or <code>codex resume</code> to continue a conversation.</li>
        </ol>
        <p>Turning off the shared server gives up:</p>
        <ul>
        <li>Codex sessions surviving a closed terminal or dropped SSH connection. Herdr already keeps your panes running, so this is usually covered.</li>
        <li>Slightly faster Codex startup, since setup happens per session again.</li>
        <li>Features that need the shared server, such as forking an open conversation.</li>
        <li>Controlling those Codex sessions from ChatGPT&#x27;s Codex Remote. If you rely on Codex Remote, keep the server on and use <code>codex --no-daemon</code> only in the panes you want in Herdcats.</li>
        <li>Codex&#x27;s self-updating background package. The <code>codex</code> you installed is used instead.</li>
        </ul>
        <p>Undo any time with <code>codex features enable daemon_auto_start</code>.</p>
        <p>A brand-new Codex session appears in Timeline after its first message. Codex starts the session when you send one.</p>
        </div>
        <div className="faq-item">
        <h4 id="using-app-does-herdcats-run-agents-on-my-iphone" className="faq-question">Does Herdcats run agents on my iPhone?</h4>
        <p>No. Agents run on your computer. Keep that computer running and reachable when you want to view their output or send input.</p>
        </div>
        <div className="faq-item">
        <h4 id="using-app-can-i-send-photos" className="faq-question">Can I send photos?</h4>
        <p>Yes, to panes on the computer you connected to directly. Uploaded photos stay on that computer until you remove them manually; the app does not remove them automatically. Their location is <code>~/.cache/herdrcat/attachments</code>.</p>
        </div>
        <div className="faq-item">
        <h4 id="using-app-what-happens-when-i-switch-apps-or-lock-my-phone" className="faq-question">What happens when I switch apps or lock my phone?</h4>
        <p>Herdcats pauses output refreshes while it is in the background. Return to the app to see refreshed output. Leaving the app does not itself stop the agents running on your computer.</p>
        </div>
        <div className="faq-item">
        <h4 id="using-app-can-i-choose-a-different-herdr-session" className="faq-question">Can I choose a different Herdr session?</h4>
        <p>Not currently. Herdcats uses Herdr&#x27;s default session, which must be running and accessible to the account you sign in with.</p>
        </div>
        <div className="faq-item">
        <h4 id="using-app-can-i-switch-between-computers-without-signing-in-to-each-one-on-my-phone" className="faq-question">Can I switch between computers without signing in to each one on my phone?</h4>
        <p>Yes. Connect Herdcats to one computer running Herdr. If Herdr on that computer has other machines connected, use the machine selector in Herdcats to switch between them and access their spaces, agents, and panes. You do not need to enter a separate SSH login in Herdcats for each of those machines.</p>
        <p>For example, connect your iPhone to Herdr on your Mac. If that Herdr session has a work server connected, you can switch between the Mac and work server from Herdcats. The Mac handles the onward connection, so it must remain running and able to reach the work server.</p>
        <p>Photos and quota information are available only for the computer your phone connected to directly.</p>
        </div>
        <div className="faq-item">
        <h4 id="using-app-how-do-i-add-another-machine" className="faq-question">How do I add another machine?</h4>
        <p>Set it up in Herdr on the computer you connect your phone to. In a terminal on that computer, run <code>herdr machine add workbox</code>, replacing <code>workbox</code> with the other computer&#x27;s SSH target, and follow the setup prompts. Herdr can offer to install Herdr on that machine if it is missing.</p>
        <p>Once configured, use Herdcats&#x27; machine selector to choose it. See <a href="https://herdr.dev/docs/connecting-machines/">Herdr&#x27;s connecting-machines guide</a> for setup and connection help.</p>
        </div>
        </div>
        </section>

        <section id="connect" aria-labelledby="connect-title" className="guide-card connection-guide">
        <h2 id="connect-title">How to connect</h2>
        <ol className="connection-steps">
        <li>Install <a href="https://herdr.dev">Herdr</a> on your computer and start its default session with the account you will use in Herdcats.</li>
        <li>Enable SSH access using the setup guide for your computer below. On a Mac, follow <a href="#macos-setup">Enable Remote Login</a>.</li>
        <li>Connect your iPhone and computer to a network where they can reach each other. Tailscale is recommended.</li>
        <li>In Herdcats, enter the hostname or IP address, SSH port, username, and password or supported private key.</li>
        <li>Compare the server fingerprint with one obtained directly from your computer or its administrator, then approve it only if they match.</li>
        </ol>
        <p className="connection-note">Already have other machines connected through Herdr? Sign in once, then use the machine selector in Herdcats to switch between them.</p>
        </section>
        {/* TAB 1: macOS */}
        <section className="tab-pane" id="macos-setup">
            <div className="guide-card">
              <h2>Setting up SSH on macOS</h2>
              <p className="card-desc">
                Follow these steps to enable SSH access on your Mac and generate an ed25519 key for Herdcats.
              </p>
              <p className="prereq-text">
                Before you start: install <a href="https://herdr.dev" target="_blank" rel="noopener noreferrer">Herdr</a> on
                the Mac and keep a Herdr session running. Herdcats drives the <code>herdr</code> CLI over SSH.
              </p>

              <ol className="step-list">
                <li className="step-item">
                  <div className="step-num">1</div>
                  <div className="step-body">
                    <h3 id="macos-setup-enable-remote-login">Enable Remote Login</h3>
                    <p>Open <strong>System Settings</strong> on your Mac:</p>
                    <p className="path-text">Apple Menu () → System Settings → General → Sharing → turn on <strong>Remote Login</strong>.</p>
                    <p className="tip-text">Ensure your user account is permitted under &ldquo;Allow access for&rdquo;.</p>
                  </div>
                </li>

                <li className="step-item">
                  <div className="step-num">2</div>
                  <div className="step-body">
                    <h3 id="macos-setup-generate-an-ed25519-key-authorize-it">Generate an Ed25519 Key &amp; Authorize It</h3>
                    <p>Open Terminal on your Mac and run this one-line command:</p>
                    <div className="code-box">
                      <pre><code>ssh-keygen -t ed25519 -f ~/.ssh/herdcats_key &amp;&amp; cat ~/.ssh/herdcats_key.pub &gt;&gt; ~/.ssh/authorized_keys &amp;&amp; chmod 700 ~/.ssh &amp;&amp; chmod 600 ~/.ssh/authorized_keys</code></pre>
                      <button
                        type="button"
                        className="copy-btn"
                        onClick={() => copyToClipboard('ssh-keygen -t ed25519 -f ~/.ssh/herdcats_key && cat ~/.ssh/herdcats_key.pub >> ~/.ssh/authorized_keys && chmod 700 ~/.ssh && chmod 600 ~/.ssh/authorized_keys', 'mac-gen')}
                      >
                        {copiedIndex === 'mac-gen' ? '✓ Copied' : 'Copy'}
                      </button>
                    </div>
                  </div>
                </li>

                <li className="step-item">
                  <div className="step-num">3</div>
                  <div className="step-body">
                    <h3 id="macos-setup-transfer-the-private-key-to-herdcats">Transfer the Private Key to Herdcats</h3>
                    <p>You can bring the private key into Herdcats using either method:</p>
                    <div className="sub-options">
                      <div className="sub-option">
                        <strong>Method A (AirDrop / Files):</strong>
                        <p>AirDrop <code>~/.ssh/herdcats_key</code> (without <code>.pub</code>) to your iPhone and save to Files. In Herdcats, tap <strong>Import…</strong> and select it.</p>
                      </div>
                      <div className="sub-option">
                        <strong>Method B (Clipboard):</strong>
                        <p>Copy to clipboard: <code>cat ~/.ssh/herdcats_key | pbcopy</code>. Paste directly into the private key field in Herdcats.</p>
                      </div>
                    </div>
                  </div>
                </li>

                <li className="step-item">
                  <div className="step-num">4</div>
                  <div className="step-body">
                    <h3 id="macos-setup-find-host-username">Find Host &amp; Username</h3>
                    <p>In Terminal on your Mac, find your connection details:</p>
                    <ul className="info-bullets">
                      <li><strong>Username:</strong> Run <code>whoami</code> (e.g. <code>alice</code>).</li>
                      <li>
                        <strong>Host:</strong> If using Tailscale (recommended), run <code>tailscale ip -4</code> or use your MagicDNS hostname (e.g. <code>mac-mini.tailnet-name.ts.net</code>). Otherwise use your local network IP (e.g. <code>192.168.1.50</code>).
                      </li>
                      <li><strong>Port:</strong> Default is <code>22</code>.</li>
                    </ul>
                  </div>
                </li>
              </ol>
            </div>
          </section>

        {/* TAB 2: Linux */}
        <section className="tab-pane" id="linux-setup">
            <div className="guide-card">
              <h2>Setting up SSH on Linux</h2>
              <p className="card-desc">
                Instructions for Ubuntu, Debian, Fedora, Arch Linux, and remote cloud VMs (AWS, GCP, Hetzner).
              </p>

              <ol className="step-list">
                <li className="step-item">
                  <div className="step-num">1</div>
                  <div className="step-body">
                    <h3 id="linux-setup-ensure-openssh-server-is-installed-running">Ensure OpenSSH Server is Installed &amp; Running</h3>
                    <p>Run the package manager commands for your distribution:</p>
                    <div className="code-box">
                      <pre><code>{`# Ubuntu / Debian
sudo apt update && sudo apt install openssh-server -y
sudo systemctl enable --now ssh

# Fedora / RHEL
sudo dnf install openssh-server -y
sudo systemctl enable --now sshd

# Arch Linux
sudo pacman -S openssh
sudo systemctl enable --now sshd`}</code></pre>
                      <button
                        type="button"
                        className="copy-btn"
                        onClick={() => copyToClipboard('sudo apt update && sudo apt install openssh-server -y && sudo systemctl enable --now ssh', 'linux-install')}
                      >
                        {copiedIndex === 'linux-install' ? '✓ Copied' : 'Copy'}
                      </button>
                    </div>
                  </div>
                </li>

                <li className="step-item">
                  <div className="step-num">2</div>
                  <div className="step-body">
                    <h3 id="linux-setup-generate-authorize-key">Generate &amp; Authorize Key</h3>
                    <p>Generate an ed25519 keypair and ensure correct permissions:</p>
                    <div className="code-box">
                      <pre><code>ssh-keygen -t ed25519 -f ~/.ssh/herdcats_key &amp;&amp; cat ~/.ssh/herdcats_key.pub &gt;&gt; ~/.ssh/authorized_keys &amp;&amp; chmod 700 ~/.ssh &amp;&amp; chmod 600 ~/.ssh/authorized_keys</code></pre>
                      <button
                        type="button"
                        className="copy-btn"
                        onClick={() => copyToClipboard('ssh-keygen -t ed25519 -f ~/.ssh/herdcats_key && cat ~/.ssh/herdcats_key.pub >> ~/.ssh/authorized_keys && chmod 700 ~/.ssh && chmod 600 ~/.ssh/authorized_keys', 'linux-keygen')}
                      >
                        {copiedIndex === 'linux-keygen' ? '✓ Copied' : 'Copy'}
                      </button>
                    </div>
                  </div>
                </li>

                <li className="step-item">
                  <div className="step-num">3</div>
                  <div className="step-body">
                    <h3 id="linux-setup-check-firewall-herdr-binary-location">Check Firewall &amp; herdr Binary Location</h3>
                    <p>If you use a firewall like UFW:</p>
                    <div className="code-box">
                      <pre><code>sudo ufw allow ssh</code></pre>
                    </div>
                    <p style={{ marginTop: '10px' }}>
                      Ensure the <code>herdr</code> CLI binary is installed and reachable in your user&rsquo;s <code>PATH</code>, <code>~/.local/bin</code>, or <code>/usr/local/bin</code>.
                    </p>
                  </div>
                </li>
              </ol>
            </div>
          </section>

        {/* TAB 3: Windows / WSL */}
        <section className="tab-pane" id="windows-setup">
            <div className="guide-card">
              <h2>Setting up SSH on Windows / WSL</h2>
              <p className="card-desc">
                For developers working on Windows, connecting via Windows Subsystem for Linux (WSL2) is the recommended path.
              </p>

              <ol className="step-list">
                <li className="step-item">
                  <div className="step-num">1</div>
                  <div className="step-body">
                    <h3 id="windows-setup-option-a-wsl-2-recommended">Option A: WSL 2 (Recommended)</h3>
                    <p>Open your WSL terminal (e.g. Ubuntu) and configure OpenSSH server:</p>
                    <div className="code-box">
                      <pre><code>{`sudo apt install openssh-server -y
sudo service ssh start
ssh-keygen -t ed25519 -f ~/.ssh/herdcats_key
cat ~/.ssh/herdcats_key.pub >> ~/.ssh/authorized_keys
chmod 700 ~/.ssh && chmod 600 ~/.ssh/authorized_keys`}</code></pre>
                      <button
                        type="button"
                        className="copy-btn"
                        onClick={() => copyToClipboard('sudo apt install openssh-server -y && sudo service ssh start && ssh-keygen -t ed25519 -f ~/.ssh/herdcats_key && cat ~/.ssh/herdcats_key.pub >> ~/.ssh/authorized_keys && chmod 700 ~/.ssh && chmod 600 ~/.ssh/authorized_keys', 'wsl-setup')}
                      >
                        {copiedIndex === 'wsl-setup' ? '✓ Copied' : 'Copy'}
                      </button>
                    </div>
                  </div>
                </li>

                <li className="step-item">
                  <div className="step-num">2</div>
                  <div className="step-body">
                    <h3 id="windows-setup-option-b-native-windows-openssh-server">Option B: Native Windows OpenSSH Server</h3>
                    <p>Open PowerShell as Administrator:</p>
                    <div className="code-box">
                      <pre><code>{`Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Start-Service sshd
Set-Service -Name sshd -StartupType 'Automatic'`}</code></pre>
                      <button
                        type="button"
                        className="copy-btn"
                        onClick={() => copyToClipboard('Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0; Start-Service sshd; Set-Service -Name sshd -StartupType \'Automatic\'', 'win-native')}
                      >
                        {copiedIndex === 'win-native' ? '✓ Copied' : 'Copy'}
                      </button>
                    </div>
                    <p style={{ marginTop: '8px' }}>
                      Authorize your public key in <code>%USERPROFILE%\.ssh\authorized_keys</code>.
                    </p>
                  </div>
                </li>
              </ol>
            </div>
          </section>

        {/* TAB 4: SSH Keys & Security */}
        <section className="tab-pane" id="ssh-keys">
            <div className="guide-card">
              <h2>SSH Key Management &amp; Security Architecture</h2>
              <p className="card-desc">
                Understanding how Herdcats handles authentication, key masking, and Keychain isolation.
              </p>

              <div className="feature-grid">
                <div className="feature-cell">
                  <h3 id="ssh-keys-why-ed25519">Why Ed25519?</h3>
                  <p>
                    Ed25519 is the key type Herdcats supports. The keys are short and quick to generate, and
                    OpenSSH has used them by default for years.
                  </p>
                </div>
                <div className="feature-cell">
                  <h3 id="ssh-keys-passphrase-notice">Passphrase Notice</h3>
                  <p>
                    Herdcats supports passphrase-protected OpenSSH ed25519 keys using AES-128-CTR or AES-256-CTR
                    with bcrypt (1–256 rounds). Set a passphrase when ssh-keygen prompts, then enter it when importing your key. Optionally remember
                    it on this device to reconnect after restarting the app.
                  </p>
                </div>
                <div className="feature-cell">
                  <h3 id="ssh-keys-on-device-keychain">On-Device Keychain</h3>
                  <p>
                    Remembered passwords and keys are stored exclusively in the iOS Keychain on your physical device.
                    Private keys and key passphrases stay on your device. SSH uses the key to sign authentication requests; passwords are sent only through the encrypted SSH connection.
                  </p>
                </div>
                <div className="feature-cell">
                  <h3 id="ssh-keys-shoulder-surfing-protection">Shoulder-Surfing Protection</h3>
                  <p>
                    Herdcats masks private key text by default, displaying only the last 2 lines for verification.
                    Tap the <strong>Show</strong> toggle if you need to inspect the full key.
                  </p>
                </div>
              </div>
            </div>
          </section>

        {/* TAB 5: Troubleshooting */}
        <section className="tab-pane" id="troubleshooting">
            <div className="guide-card">
              <h2>Troubleshooting &amp; Common Issues</h2>
              <p className="card-desc">
                Quick resolutions for connection, authentication, and environment issues.
              </p>

              <div className="faq-list">
                <div className="faq-item">
                  <h3 id="troubleshooting-authentication-failed-check-username-and-password-key"><TriangleAlert className="warn-icon" size={18} strokeWidth={2.25} aria-hidden="true" /><span>&ldquo;Authentication failed — check username and password/key&rdquo;</span></h3>
                  <p>
                    The SSH server rejected authentication (<code>allAuthenticationOptionsFailed</code>).
                  </p>
                  <ul>
                    <li>Verify the matching public key shown in Herdcats is in <code>~/.ssh/authorized_keys</code>.</li>
                    <li>
                      Check permissions on your remote host:
                      <br />
                      <code>chmod 700 ~/.ssh &amp;&amp; chmod 600 ~/.ssh/authorized_keys</code>
                    </li>
                    <li>Ensure you imported the <strong>private key</strong> (e.g. <code>herdcats_key</code>), not the <code>.pub</code> public key.</li>
                  </ul>
                </div>

                <div className="faq-item">
                  <h3 id="troubleshooting-host-key-verification-failed-changed"><TriangleAlert className="warn-icon" size={18} strokeWidth={2.25} aria-hidden="true" /><span>&ldquo;Host key verification failed / changed&rdquo;</span></h3>
                  <p>
                    Herdcats pins remote host key fingerprints in Keychain on first connect (Trust On First Use). If your server was reinstalled or rebuilt, the key will fail closed to prevent MITM attacks. Verify the change directly with your computer or its administrator before restoring access. The app shows both the previously approved and new fingerprints. After verifying the new fingerprint through a trusted channel, confirm verification and choose “Replace Approved Key and Connect” to update trust for that hostname and port.
                  </p>
                </div>

                <div className="faq-item">
                  <h3 id="troubleshooting-connection-timeout-host-unreachable"><TriangleAlert className="warn-icon" size={18} strokeWidth={2.25} aria-hidden="true" /><span>Connection Timeout / Host Unreachable</span></h3>
                  <p>
                    If connecting over Wi-Fi, ensure your phone and computer are on the same subnet. For remote access across networks, install <strong>Tailscale</strong> on both devices to connect by MagicDNS hostname.
                  </p>
                </div>

                <div className="faq-item">
                  <h3 id="troubleshooting-herdr-command-not-found"><TriangleAlert className="warn-icon" size={18} strokeWidth={2.25} aria-hidden="true" /><span>&ldquo;herdr: command not found&rdquo;</span></h3>
                  <p>
                    Herdcats looks for <code>herdr</code> in <code>PATH</code>, <code>~/.local/bin</code>, <code>/opt/homebrew/bin</code>, and <code>/usr/local/bin</code>. Ensure Herdr is installed on the remote machine.
                  </p>
                </div>

                <div className="faq-item">
                  <h3 id="troubleshooting-timeline-helper-missing-or-install-failed"><TriangleAlert className="warn-icon" size={18} strokeWidth={2.25} aria-hidden="true" /><span>&ldquo;Timeline says Herdcats Bridge is missing or could not be installed&rdquo;</span></h3>
                  <p>
                    Timeline needs optional Herdcats Bridge (<code>herdcats-bridge</code>) on the computer your phone connects to directly — without it, that view falls back to Terminal, which stays fully usable. Tap <strong>Install</strong> in the setup sheet, or copy the manual install command shown there and run it on that computer, then tap <strong>Check again</strong>. For an existing installation, tap <strong>Upgrade</strong>; manual upgrades require <code>--replace</code>. If installation cannot be verified, choose <strong>Check installation</strong>; Terminal and Compose keep working without Herdcats Bridge.
                  </p>
                </div>
                <div className="faq-item">
                  <h4 id="troubleshooting-codex-session-not-identified" className="faq-question">Timeline says &ldquo;Send a message to start&rdquo; or &ldquo;Codex session not identified&rdquo;</h4>
                  <p><strong>Send a message to start:</strong> Codex starts a session only when you send its first message. Send one, and Timeline appears once Codex replies.</p>
                  <p><strong>Codex session not identified:</strong> if Timeline says the shared background server is running, follow <a href="#using-app-codex-shared-background-server">Codex background server</a>, then restart Codex in that pane.</p>
                  <p>If the server is already off, the pane&#x27;s session reference is invalid or matches more than one session. Restart Codex in that pane, then tap <strong>Check again</strong>.</p>
                  <p>Codex sessions started before you turned the server off keep using it, and stop working once it is stopped. Quit Codex in that pane and run <code>codex resume</code> to continue the conversation in Timeline.</p>
                </div>
              </div>
            </div>
          </section>

        </div>
      </main>

      <SiteFooter />

      {/* Styles */}
      <style jsx>{`
        .support-wrapper {
          min-height: 100dvh;
          height: 100dvh;
          background: var(--bg);
          --content-width: 1160px;
          color: var(--text);
          font-family: var(--font-sans), Futura, "Century Gothic", system-ui, sans-serif;
          overflow-y: auto;
          -webkit-overflow-scrolling: touch;
          display: flex;
          flex-direction: column;
        }






        .support-main {
          flex: 1;
          max-width: 1200px;
          display: grid;
          grid-template-columns: 220px minmax(0, 1fr);
          align-items: start;
          gap: 40px;
          width: 100%;
          margin: 0 auto;
          padding: 36px 20px 60px;
        }

        .hero-section {
          margin-bottom: 32px;
        }



        .security-banner {
          display: flex;
          align-items: center;
          border-left: 3px solid var(--accent);
          padding-left: 12px;
          font-size: 13px;
          color: var(--text);
        }

        .ts-tag {
          display: inline-block;
          margin-left: 10px;
          background: var(--sign-bg);
          color: var(--warm);
          border: 2px solid var(--warm);
          font-family: var(--font-pixel), monospace;
          font-size: 10px;
          letter-spacing: 0.05em;
          text-transform: uppercase;
          padding: 2px 6px;
          border-radius: 4px;
        }

        /* Tabs Bar */
        .tabs-bar {
          display: flex;
          gap: 8px;
          overflow-x: auto;
          padding-bottom: 4px;
          margin-bottom: 24px;
          border-bottom: 1px solid var(--border);
        }

        .tab-btn {
          background: transparent;
          border: none;
          color: var(--text-muted);
          font-size: 14px;
          font-weight: 600;
          padding: 10px 16px;
          border-radius: 8px 8px 0 0;
          cursor: pointer;
          white-space: nowrap;
          transition: all 0.15s ease;
          border-bottom: 2px solid transparent;
        }

        .tab-btn:hover {
          color: var(--text);
        }

        .tab-btn.active {
          color: var(--text);
          border-bottom: 2px solid var(--accent);
          background: var(--raised);
        }

        /* Guide sections */
        .guide-card {
        }

        .guide-card h2 {
          font-size: 22px;
          font-weight: 700;
          color: var(--text);
          margin-bottom: 6px;
        }

        .card-desc {
          font-size: 14px;
          color: var(--text-muted);
          margin-bottom: 24px;
        }

        .prereq-text {
          font-size: 13px;
          color: var(--text);
          margin: -12px 0 24px;
        }

        .prereq-text a {
          color: var(--accent);
        }

        /* Steps */
        .step-list {
          list-style: none;
          display: flex;
          flex-direction: column;
          gap: 24px;
        }

        .step-item {
          display: flex;
          gap: 16px;
        }

        .step-num {
          width: 32px;
          height: 32px;
          border-radius: 4px;
          background: var(--raised);
          color: var(--accent);
          font-family: var(--font-pixel), monospace;
          font-size: 14px;
          display: flex;
          align-items: center;
          justify-content: center;
          flex-shrink: 0;
          margin-top: 2px;
        }

        .step-body {
          flex: 1;
        }

        .step-body h3 {
          font-size: 16px;
          font-weight: 600;
          color: var(--text);
          margin-bottom: 6px;
        }

        .step-body p {
          font-size: 14px;
          color: var(--text);
          line-height: 1.5;
          margin-bottom: 8px;
        }

        .path-text {
          font-size: 13px;
          color: var(--accent) !important;
        }

        .tip-text {
          font-size: 12px !important;
          color: var(--text-muted) !important;
          margin-top: 4px;
        }

        /* Code box */
        .code-box {
          position: relative;
          margin: 8px 0;
          overflow-x: auto;
          padding: 12px 76px 12px 14px;
          background: var(--bg-deep);
          border: 1px solid var(--border);
          border-radius: 8px;
        }

        .code-box pre {
          margin: 0;
          font-family: ui-monospace, "SF Mono", Menlo, Monaco, Consolas, monospace;
          font-size: 12px;
          color: var(--accent);
          line-height: 1.5;
          white-space: pre-wrap;
          word-break: break-all;
        }

        .copy-btn {
          position: absolute;
          top: 10px;
          right: 10px;
          background: var(--raised);
          color: var(--text);
          font-family: inherit;
          border: none;
          font-size: 11px;
          font-weight: 600;
          padding: 4px 10px;
          border-radius: 6px;
          cursor: pointer;
          transition: background 0.15s ease;
        }

        .copy-btn:hover {
          background: var(--accent-fill);
          color: var(--on-accent);
        }

        .sub-options {
          display: grid;
          grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
          gap: 12px;
          margin-top: 8px;
        }

        .sub-option {
        }

        .sub-option strong {
          display: block;
          font-size: 13px;
          color: var(--accent);
          margin-bottom: 4px;
        }

        .sub-option p {
          font-size: 12px;
          color: var(--text-muted);
          margin: 0;
        }

        .info-bullets {
          list-style: none;
          display: flex;
          flex-direction: column;
          gap: 8px;
          margin-top: 8px;
        }

        .info-bullets li {
          font-size: 13px;
          color: var(--text);
        }

        /* Feature grid */
        .feature-grid {
          display: grid;
          grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
          gap: 16px;
        }

        .feature-cell {
        }

        .feature-cell h3 {
          font-size: 15px;
          font-weight: 700;
          color: var(--accent);
          margin-bottom: 6px;
        }

        .feature-cell p {
          font-size: 13px;
          color: var(--text-muted);
          line-height: 1.5;
        }


        .connection-guide, .faq-section { margin-bottom: 48px; }
        .faq-section { margin-top: 24px; }
        .connection-steps { padding-left: 22px; color: var(--text); font-size: 14px; line-height: 1.6; }
        .connection-steps li { margin-bottom: 12px; }
        .connection-note { margin-top: 16px; color: var(--text-muted); font-size: 14px; line-height: 1.6; }
        .connection-guide a, .faq-item a { color: var(--accent); text-decoration: underline; }
        .faq-category { margin: 24px 0 12px; font-size: 17px; color: var(--text); }
        .faq-question { font-size: 15px; font-weight: 600; line-height: 1.5; color: var(--text); margin-bottom: 12px; }
        .faq-item ol { padding-left: 18px; font-size: 13px; color: var(--text); line-height: 1.6; }
        .faq-item li { margin-bottom: 6px; }
        .faq-item { min-width: 0; overflow-wrap: anywhere; }
        .step-body { min-width: 0; }

        .support-sections { display: flex; flex-wrap: wrap; gap: 16px; margin-bottom: 24px; }
        .support-sections a { color: var(--accent); text-decoration: underline; }


        .support-wrapper { scroll-behavior: smooth; }
        .support-content { min-width: 0; }
        .support-content section, .support-content h3, .support-content h4 { scroll-margin-top: 40px; }
        .support-sidebar { position: sticky; top: 32px; max-height: calc(100dvh - 130px); overflow-y: auto; }
        .sidebar-title { color: var(--accent); font-family: var(--font-pixel), monospace; font-size: 12px; letter-spacing: 0.05em; text-transform: uppercase; margin: 0 0 16px 12px; }
        .section-link { display: block; padding: 10px 12px; border-left: 2px solid var(--border); color: var(--text-muted); font-size: 14px; text-decoration: none; }
        .subsection-link { padding: 8px 12px 8px 24px; font-size: 12px; line-height: 1.5; }
        .section-link:hover { color: var(--text); }
        .section-link.active { color: var(--text); border-left-color: var(--accent); background: var(--raised); }
        .section-link:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
        .tab-pane { margin-bottom: 48px; }
        @media (max-width: 800px) {
          .support-main { grid-template-columns: minmax(0, 1fr); gap: 24px; padding-top: 16px; }
          .support-sidebar { top: 0; z-index: 30; max-height: none; background: var(--bg); padding: 8px 0; border-bottom: 1px solid var(--border); }
          .support-sidebar nav { display: flex; overflow-x: auto; }
          .sidebar-title { display: none; }
          .section-link { flex-shrink: 0; border-left: 0; border-bottom: 2px solid transparent; }
          .section-link.active { border-bottom-color: var(--accent); }
          .support-content section, .support-content h3, .support-content h4 { scroll-margin-top: 90px; }
        }
        @media (prefers-reduced-motion: reduce) {
          .support-wrapper { scroll-behavior: auto; }
        }

        /* FAQ */
        .faq-list {
          display: flex;
          flex-direction: column;
          gap: 28px;
        }

        .faq-item {
        }

        .faq-item h3 {
          display: flex;
          align-items: flex-start;
          gap: 8px;
          font-size: 15px;
          font-weight: 600;
          color: var(--text);
          margin-bottom: 8px;
        }

        .faq-item p {
          font-size: 13px;
          color: var(--text-muted);
          line-height: 1.5;
          margin-bottom: 8px;
        }

        .faq-item ul {
          padding-left: 18px;
          font-size: 13px;
          color: var(--text);
          line-height: 1.6;
        }

        .faq-item :global(.warn-icon) {
          flex-shrink: 0;
          margin-top: 2px;
          color: var(--accent);
        }

        /* Footer */





        @media (max-width: 640px) {
          .sub-options, .feature-grid {
            grid-template-columns: 1fr;
          }
        }
      `}</style>
    </div>
  );
}
