# Homebrew formula for OpenCodeRust.
#
# Builds from source rather than downloading a prebuilt tarball, because release
# artifacts are not published yet for all platforms. Homebrew already has the Rust
# toolchain as a build dependency, so `brew install` does the compile that `npm`
# deliberately refuses to do inside a postinstall script.

class Opr < Formula
  desc "The open source AI coding agent, in Rust"
  homepage "https://github.com/vakandi/opencode-rust"
  url "https://github.com/vakandi/opencode-rust.git",
      revision: 1
  version "0.1.0"
  license "MIT"
  head "https://github.com/vakandi/opencode-rust.git", branch: "main"

  depends_on "rust" => :build
  depends_on "pkg-config" => :build
  # openssl-sys / onig_sys link against system libraries rather than vendoring them.
  depends_on "openssl" => :build
  depends_on "node" => :recommended

  def install
    system "cargo", "install", "--path", ".", "--bin", "opencode",
           "--root", staging_dir/"cargo-install", "--locked"
    bin.install Dir[staging_dir/"cargo-install"/"bin"/"opencode"]
    # `opr` is a real symlink here, unlike the npm package where it must be a
    # dispatching script: Homebrew owns a single, immutable binary per prefix.
    ln_sf bin/"opencode", bin/"opr"
  end

  def caveats
    <<~EOS
      The `opencode` binary is installed. A shorter `opr` alias is also created:

        opr                  # interactive TUI (same as `opencode tui`)
        opr run "your task"  # one prompt, runs to completion
        opr serve --port 3000

      Note: `opr --some-flag` becomes `opencode tui --some-flag`. Only a known
      subcommand is forwarded verbatim; anything else falls through to the TUI.

      First run needs a provider key, e.g. `export ANTHROPIC_API_KEY=...`.
      Configuration lives in ~/.config/opencode/opencode.json
    EOS
  end

  test do
    assert_match "OpenCode", shell_output("#{bin}/opencode version")
  end
end