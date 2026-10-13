# Homebrew formula for OpenCodeRust — https://github.com/vakandi/opencode-rust
#
# Builds from source rather than downloading a prebuilt tarball, because release
# artifacts are not published yet for all platforms. Homebrew already pulls the Rust
# toolchain as a build dependency, so `brew install` does the compile that `npm`
# deliberately refuses to do inside a postinstall script.

class OpencodeRust < Formula
  desc "The open source AI coding agent, in Rust"
  homepage "https://github.com/vakandi/opencode-rust"
  url "https://github.com/vakandi/opencode-rust.git",
      revision: 1
  version "0.1.0"
  license "MIT"
  head "https://github.com/vakandi/opencode-rust.git", branch: "main"

  depends_on "rust" => :build
  depends_on "pkg-config" => :build
  # openssl-sys and onig_sys link against system libraries rather than vendoring them.
  depends_on "openssl" => :build
  depends_on "node" => :recommended

  def install
    system "cargo", "install", "--path", ".", "--bin", "opencode",
           "--root", staging_dir/"cargo-install", "--locked"
    bin.install Dir[staging_dir/"cargo-install"/"bin"/"opencode"]
    # `opr` is a plain symlink here, unlike the npm package where it has to be a
    # dispatching script. That is why `opr --flag` behaves differently between the two
    # installs: through npm it becomes `opencode tui --flag`, through brew it becomes
    # `opencode --flag` and is rejected. Give people the `tui` subcommand explicitly.
    ln_sf bin/"opencode", bin/"opr"
  end

  def caveats
    <<~EOS
      Installed: opencode, plus an `opr` alias for the same binary.

        opr                      # interactive TUI
        opr run "your task"      # one prompt, runs to completion
        opr serve --port 3000

      Unlike the npm install, `opr` here is a symlink, not a dispatcher. Passing a
      global flag (`opr --some-flag`) forwards it to opencode unchanged and will be
      rejected. Name the subcommand instead: `opr tui --some-flag`.

      First run needs a provider key, e.g. `export ANTHROPIC_API_KEY=...`.
      Configuration lives in ~/.config/opencode/opencode.json
    EOS
  end

  test do
    assert_match "OpenCode", shell_output("#{bin}/opencode version")
  end
end