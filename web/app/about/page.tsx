import { SitePage, sitePageStyles as styles } from '@/components/SitePage';

export const metadata = {
  title: 'Herdcats — About',
  description: 'Meet Kenny, the maker of Herdcats, and his company enchantinglabs. Learn why this minimal, open source iPhone client for Herdr exists.',
};

export default function AboutPage() {
  return (
    <SitePage current="about" title="About Herdcats">
      <section className={styles.section}>
        <h2 className={styles.label}>Who</h2>
        <p>
          Hi, I&rsquo;m Kenny, the maker of Herdcats. enchantinglabs is my company, where I build and
          maintain Herdcats.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>Why</h2>
        <p>
          <a href="https://herdr.dev" target="_blank" rel="noopener noreferrer">Herdr</a> changed how I
          run AI agents across machines. When I&rsquo;m away from my desk, I still want to check on them.
          Existing tools were subscriptions, closed source, or locked to a few popular agents. I wanted
          one that talks directly to each agent&rsquo;s own terminal UI, whatever the agent.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>What</h2>
        <p>Herdcats is a minimal iPhone app for Herdr:</p>
        <ul className={styles.list}>
          <li>Direct SSH to your machine, with no relay — the optional Herdcats Bridge runs on your own computer, only while you use it</li>
          <li>Navigation that follows Herdr&rsquo;s own spaces, tabs, panes and agents</li>
          <li>An optional quota view, if <code>quota-axi</code> is installed on the connected machine</li>
        </ul>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>Why Herdcats?</h2>
        <p>
          &ldquo;Herding cats&rdquo; means trying to organize things that won&rsquo;t be organized, which is
          how running several independent AI agents can feel. The name also nods to{' '}
          <a href="https://herdr.dev" target="_blank" rel="noopener noreferrer">Herdr</a>
          &rsquo;s herd, and to the lazy pixel cats that show up when you&rsquo;re waiting or offline.
        </p>
      </section>

      <section className={styles.section}>
        <h2 className={styles.label}>How to get it</h2>
        <p>
          It&rsquo;s{' '}
          <a href="https://github.com/zhiyao/herdcats" target="_blank" rel="noopener noreferrer">
            open source
          </a>{' '}
          and contributions are welcome. It&rsquo;s on TestFlight now and coming to the App Store for a
          one-time price of US$9.99.
        </p>
      </section>
    </SitePage>
  );
}
