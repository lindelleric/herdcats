# Preparing a release

## First public source preview

The first public source release uses the existing app version **0.3.1**
(build **84**) and tag **herdrcat-v0.3.1**, matching the repository's version
helpers. It is a GitHub prerelease containing source, not a signed app binary.
The existing version does not need a bump just to tag the public baseline.
Its proposed notes are in [RELEASE_NOTES.md](RELEASE_NOTES.md).

## Checklist

- Use the public repository's `main`; do not import private branches or history.
- Merge release changes through a PR with passing **iOS** and **Web** checks.
- Confirm the chosen commit on `main` has passed CI. Review open security alerts.
- Check README build commands, limitations, security-reporting links, and notices.
- Keep dependency notices consistent with committed locks. Document any unresolved
  artwork, provider-term, or redistribution questions in the release notes.
- Confirm `ios/project.yml`, the generated project, and Info.plist agree on version.
- For source previews, describe what was tested and that live SSH UI tests were
  not run by CI. Attach no credentials, captures of private sessions, or unsigned
  binaries presented as installable apps.

Create a **draft prerelease** at the exact tested commit, review it, then publish
when ready. GitHub supplies source archives when the release is published.
Do not tag a moving branch or an unmerged PR commit.

```sh
# Replace TESTED_COMMIT_SHA with the full commit SHA checked above.
gh release create herdrcat-v0.3.1 \
  --target TESTED_COMMIT_SHA --draft --prerelease \
  --title 'Herdcats 0.3.1 — first public source preview' \
  --notes-file RELEASE_NOTES.md
```

## Later versions (app + TestFlight)

Use the two-step deploy helpers. CI still gates the bump before any tag or upload.

```sh
# On up-to-date main, clean tree:
./ios/bin/deploy prepare
# → creates release/X.Y.Z, bumps version/build, pushes, opens PR

# After the PR is merged and CI is green:
git checkout main && git pull
export DEVELOPMENT_TEAM=YOURTEAMID
./ios/bin/deploy publish
# → tags herdcats-vX.Y.Z, changelog, TestFlight upload, pushes tag
```

New tags use the `herdcats-v*` prefix. Helpers still recognize legacy `herdrcat-v*`
tags when computing the next version or changelog ranges.

`publish` requires local `main` to match `origin/main` when `origin` is configured
(push merged commits before publishing). If a step fails after the local tag is
created, the script prints recovery steps. Before the TestFlight upload has run,
retry with `./ios/bin/changelog` (if needed), `./ios/bin/beta`, and
`git push origin herdcats-vX.Y.Z`. Once the upload already completed, do not
rerun `bin/beta` — the same CFBundleVersion would be rejected as a duplicate —
just finish with `git push origin herdcats-vX.Y.Z`. Alternatively remove the
local tag with `git tag -d herdcats-vX.Y.Z` and run `publish` again.

`ios/bin/release` remains the bump-only helper (release branch required, no tag).
`ios/bin/changelog` and `ios/bin/beta` remain runnable on their own.

Fresh public history has fewer commits than the old private repository. The bump
helper preserves build-number monotonicity via
`max(CFBundleVersion+1, commitCount+1)`. For App Store or TestFlight uploads,
also check the highest build already uploaded in App Store Connect if uploads
came from another machine or history.

App Store **review** submission stays manual in App Store Connect after the
TestFlight build is processed.

GitHub source releases (draft prerelease at a tested SHA) remain separate from
signed TestFlight builds; use the checklist above for source previews.

## Local signing for TestFlight

Keep the shared `ios/project.yml` and generated project free of a personal
`DEVELOPMENT_TEAM` setting. XcodeGen overwrites manual project edits during
release preparation. The TestFlight lane reads the team from your environment
and passes it to the archive and IPA export without storing it in the project:

```sh
export DEVELOPMENT_TEAM=YOURTEAMID
./ios/bin/beta
```

Replace `YOURTEAMID` with the 10-character Team ID from your Apple membership.
The Team ID identifies the membership; it is not an authentication credential.
You still need authorized Apple authentication and signing credentials. Keep
API private keys and Apple sessions outside Git. Existing
`APP_STORE_CONNECT_API_KEY_*` environment variables remain supported; without
an API key, fastlane falls back to Apple session authentication.

`./ios/bin/deploy prepare` does not need Apple credentials.
`./ios/bin/deploy publish` (and `./ios/bin/beta`) require `DEVELOPMENT_TEAM` and
authorized App Store Connect authentication.
