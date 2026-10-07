# Releasing Tayga

The go-live and release checklist. Until step 1 has run once, the README's default commands fail: the installer resolves `/releases/latest` and exits with "no release found", `helm install … --version 0.1.0` finds no chart, and the release badge shows "no releases". Do not announce before step 6.

## Before you start

- The release branch is merged into `master`. The installer one-liner downloads `scripts/install.sh` from `master`, and the docs site deploys from `master`.
- The workspace version in `Cargo.toml` matches the tag you are about to push. The release workflow checks this and stops if they differ.
- CI is green on the commit you will tag.

## 1. Tag the release

```sh
git checkout master && git pull
git tag -a v0.1.0 -m "v0.1.0"
git push origin v0.1.0
```

The tag starts `.github/workflows/release.yml`, which:

- checks the tag against the version in `Cargo.toml`;
- builds the image for `linux/amd64` and `linux/arm64` and pushes the manifest list `ghcr.io/softberries/tayga` with the tags `0.1.0`, `latest` (not for pre-releases such as `v0.2.0-rc.1`) and `sha-<short>`;
- lints and packages the Helm chart and pushes it to `oci://ghcr.io/softberries/charts/tayga`;
- creates the GitHub release with `tayga-standalone-0.1.0.tar.gz`, `tayga-standalone-0.1.0.tar.gz.sha256` and the chart `tayga-0.1.0.tgz`.

Watch the run in the Actions tab until every job is green.

## 2. Verify the release assets

```sh
gh release view v0.1.0 --json assets --jq '.assets[].name'
# tayga-0.1.0.tgz
# tayga-standalone-0.1.0.tar.gz
# tayga-standalone-0.1.0.tar.gz.sha256

gh release download v0.1.0 --pattern 'tayga-standalone-*' --dir /tmp/tayga-release
(cd /tmp/tayga-release && shasum -a 256 -c tayga-standalone-0.1.0.tar.gz.sha256)

docker buildx imagetools inspect ghcr.io/softberries/tayga:0.1.0   # lists linux/amd64 and linux/arm64
helm show chart oci://ghcr.io/softberries/charts/tayga --version 0.1.0
```

## 3. Make the GHCR packages public

New GHCR packages are private. In GitHub, open the organisation's **Packages**, then for each of `tayga` and `charts/tayga`: **Package settings → Danger Zone → Change visibility → Public**.

Check from a machine with no GHCR login (`docker logout ghcr.io`, `helm registry logout ghcr.io`):

```sh
docker pull ghcr.io/softberries/tayga:0.1.0
helm pull oci://ghcr.io/softberries/charts/tayga --version 0.1.0
```

## 4. Publish the docs site

1. **Settings → Pages → Build and deployment → Source: GitHub Actions.**
2. Run the **Docs site** workflow (`.github/workflows/pages.yml`) with **Run workflow**, or push a change under `site/**` to `master`.
3. Confirm the deploy job is green and that https://softberries.github.io/tayga/ loads, including a deep link such as https://softberries.github.io/tayga/getting-started/quickstart/ and the video at https://softberries.github.io/tayga/#tour. The README's links into the site return 404 until this first deploy.

## 5. Smoke-test the README commands on a clean machine

Use a machine or account with no Tayga checkout, no GHCR login and nothing listening on ports 8090, 4317 and 4318.

```sh
curl -fsSL https://raw.githubusercontent.com/softberries/tayga/master/scripts/install.sh | sh
```

The installer should verify the checksum, start the stack and print the URLs. Open http://localhost:8090, send a few traces (see the [Quickstart](https://softberries.github.io/tayga/getting-started/quickstart/)) and check that stories appear. Remove it with `sh ~/tayga/install.sh --uninstall --purge`.

On a Kubernetes cluster (kind is enough):

```sh
helm install tayga oci://ghcr.io/softberries/charts/tayga --version 0.1.0 \
  --namespace tayga --create-namespace --wait
kubectl -n tayga port-forward svc/tayga-api 8090:8090
```

Then check the README renders on GitHub: the hero image, the badges (release, CI, docs) and the video poster.

## 6. Announce

Only after steps 1 to 5 pass.

## Later releases

Bump the workspace version in `Cargo.toml` and the chart's `version` and `appVersion` in `deploy/helm/tayga/Chart.yaml`, update `site/src/content/docs/changelog.mdx`, merge, then repeat steps 1, 2 and 5. Steps 3 and 4 are one-time.
