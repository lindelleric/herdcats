#!/usr/bin/env bash
# Tag, build and publish a Herdcats Bridge release from origin/main.
# Usage: bridge-release-tag.sh 0.2.1   (run inside a herdcats checkout)
set -euo pipefail

VERSION="${1:?usage: $0 X.Y.Z}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must be X.Y.Z"; exit 1; }
TAG="herdcats-bridge-v$VERSION"
REPO="zhiyao/herdcats"
confirm() { read -r -p "$1 [y/N] " a; [[ "$a" == y || "$a" == Y ]] || { echo "Stopped."; exit 1; }; }

echo "== Fetching origin"
git fetch -q origin --tags
SHA=$(git rev-parse origin/main)
echo "origin/main: $(git log --oneline -1 "$SHA")"

echo "== Checking versions on origin/main"
CARGO=$(git show "$SHA:bridge/Cargo.toml" | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)
INSTALL=$(git show "$SHA:bridge/scripts/install.sh" | sed -n 's/^HB_VERSION=//p' | head -1)
[[ "$CARGO" == "$VERSION" ]] || { echo "Cargo.toml is $CARGO, not $VERSION"; exit 1; }
[[ "$INSTALL" == "$VERSION" ]] || { echo "install.sh is $INSTALL, not $VERSION"; exit 1; }
echo "Cargo.toml and install.sh: $VERSION"

if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null; then
  echo "$TAG already exists on origin"; exit 1
fi

echo "== Checking CI on $SHA"
CI=$(gh run list -R "$REPO" --commit "$SHA" --json name,status,conclusion \
      --jq '.[] | "\(.name)\t\(.status)\t\(.conclusion)"')
echo "$CI"
[[ -n "$CI" ]] || { echo "No CI runs found for $SHA yet"; exit 1; }
if echo "$CI" | awk -F'\t' '$2!="completed" || ($3!="success" && $3!="skipped" && $3!="neutral")' | grep -q .; then
  echo "CI is not all green yet. Re-run this script once it finishes."; exit 1
fi

confirm "Create annotated $TAG on ${SHA:0:7} and push it?"
git tag -a "$TAG" "$SHA" -m "herdcats-bridge $VERSION"
git push origin "$TAG"

echo "== Waiting for the release workflow"
RUN=""
for _ in $(seq 1 30); do
  RUN=$(gh run list -R "$REPO" --workflow bridge-release.yml --commit "$SHA" --limit 1 --json databaseId --jq '.[0].databaseId // empty')
  [[ -n "$RUN" ]] && break
  sleep 5
done
[[ -n "$RUN" ]] || { echo "Release run didn't start; check Actions"; exit 1; }
gh run watch -R "$REPO" "$RUN" --exit-status

echo "== Checking the draft release"
gh release view "$TAG" -R "$REPO" --json isDraft,assets --jq '"draft: \(.isDraft)", (.assets[].name)'
for a in herdcats-bridge-darwin-arm64 herdcats-bridge-darwin-x86_64 herdcats-bridge-linux-arm64 herdcats-bridge-linux-x86_64 install.sh SHA256SUMS; do
  gh release view "$TAG" -R "$REPO" --json assets --jq '.assets[].name' | grep -qx "$a" || { echo "Missing asset: $a"; exit 1; }
done
REL=$(gh release download "$TAG" -R "$REPO" -p install.sh -O - | shasum -a 256 | cut -d' ' -f1)
SRC=$(git show "$SHA:bridge/scripts/install.sh" | shasum -a 256 | cut -d' ' -f1)
[[ "$REL" == "$SRC" ]] || { echo "Released install.sh ($REL) differs from the repo ($SRC)"; exit 1; }
echo "install.sh sha256: $REL (matches the repo)"

confirm "Publish $TAG (make it public)?"
gh release edit "$TAG" -R "$REPO" --draft=false
echo "Published: https://github.com/$REPO/releases/tag/$TAG"
echo "installerSHA256 for the pin bump: $REL"
