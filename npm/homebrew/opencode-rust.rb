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
      revision: "0c72724ddb4b564249fdb062fdac57baa9cedcd8"
  version "0.1.4"
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
    # Install the built binary as `opr` and nothing else. The crate still names its
    # binary `opencode`, but `opencode` belongs to the upstream CLI
    # (https://github.com/anomalyco/opencode) — writing that name into Homebrew's bin
    # directory silently shadows a user's real opencode installation and breaks every
    # other formula that depends on it. Renaming on install keeps this formula inert
    # with respect to the upstream tool.
    bin.install staging_dir/"cargo-install"/"bin"/"opencode" => "opr"
  end

  def caveats
    <<~EOS
      Installed as `opr`. This formula does not install a binary named `opencode`, so
      it cannot shadow the upstream opencode CLI.

        opr                      # interactive TUI
        opr run "your task"      # one prompt, runs to completion
        opr serve --port 3000

      `opr` here is the binary itself, not the npm package's dispatching script. A
      global flag (`opr --some-flag`) is forwarded unchanged and will be rejected by
      the CLI — name the subcommand instead: `opr tui --some-flag`.

      First run needs a provider key, e.g. `export ANTHROPIC_API_KEY=...`.
      Configuration lives in ~/.config/opencode/opencode.json
    EOS
  end

  test do
    assert_match "OpenCode", shell_output("#{bin}/opr version")
    # Guards the rename above: if anyone re-adds an `opencode` binary, this fails.
    refute_path_exists bin/"opencode"
  end
end