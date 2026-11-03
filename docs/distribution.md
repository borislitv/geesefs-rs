# Dependency notices for a binary distribution

Filesystem and performance acceptance remain open. A successful notice scan does
not authorize a release or establish compatibility. The current notice policy
targets `x86_64-unknown-linux-gnu`; review and regenerate for another target.

Use public `cargo-about` 0.9.3 and the checked-in `about.toml` / `about.hbs`.
Generation uses the locked dependency graph, a conservative all-feature superset,
and upstream license files. Preserve complete package sources with Cargo's
existing vendor command so native assembly/C and individual-file notices are not
reduced to a crate's manifest license expression.

```bash
out=benchmark-results/private/notice-bundle
mkdir -p -m 700 benchmark-results/private
test ! -e "$out"
mkdir -m 700 "$out"
timeout 180s cargo about generate --locked --all-features --fail \
  --config about.toml --output-file "$out/licenses.html" about.hbs
timeout 300s cargo vendor --locked --versioned-dirs "$out/dependency-sources" >/dev/null
cp LICENSE NOTICE Cargo.lock "$out/"
```

Keep the executable, its exact source/build manifest and SHA256, `LICENSE`,
`NOTICE`, `Cargo.lock`, `licenses.html` and `dependency-sources` together in the
distribution. Keep raw scanner JSON private: it includes local source paths.
The HTML template exports only package names, versions and license text.
Do not overwrite an existing evidence directory or treat a source audit as
verification of a different executable or lockfile.

The locked x86_64 Linux verification generates the HTML and retains 338 dependency
packages. Fifteen selected native/embedded notice files below match their upstream
crate-source SHA256 values byte-for-byte; the HTML contains no private checkout or
registry paths. The default release executable and compiled-source/build hashes
are retained with that bundle. This verifies this packaging input, not another
target, changed dependency graph or filesystem acceptance.

## Native and embedded notice review

| Locked component | Notices retained in its source package |
| --- | --- |
| `tikv-jemalloc-sys 0.7.1+5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8` | Wrapper MIT/Apache-2.0 files and `jemalloc/COPYING` (BSD-2-Clause; Jason Evans, Mozilla, Facebook). |
| `aws-lc-rs 1.18.2`, `aws-lc-sys 0.46.0` | Root and bundled `aws-lc/LICENSE`, covering AWS-LC/BoringSSL/OpenSSL and embedded arithmetic/entropy code; retain third-party files and file-level notices. Use the applicable permissive alternatives identified there. |
| `ring 0.16.20` | Full `LICENSE`, including ISC, OpenSSL/SSLeay acknowledgments and the compiled fiat-crypto MIT notice. |
| `ring 0.17.14` | `LICENSE`, `LICENSE-other-bits`, `LICENSE-BoringSSL`, once_cell MIT/Apache texts and file-level notices. |
| `webpki 0.22.4` | ISC `LICENSE`, Chromium BSD-3-Clause and bettertls Apache-2.0 notices. Checksum-specific clarification retains all three; no scanner-inferred Pixar license is accepted. |

The retained dependency sources include build/test material as well as linked
code; this is an attribution superset, not a claim that every package/file is
linked into the executable. AWS-LC itself distinguishes compiled third-party
code from test/build-only material. Repeat the review when the lockfile, target,
native build features or packaged contents change. Do not remove or relabel
upstream source notices to make a license scanner pass.
