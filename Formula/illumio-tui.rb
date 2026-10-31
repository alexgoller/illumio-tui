# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.5.8"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.8/illumio-tui-0.5.8-aarch64-apple-darwin.tar.gz"
      sha256 "2e2c79002286c7316cc99d2a41717c19249410b780709dadf594ae2a3bf49133"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.8/illumio-tui-0.5.8-x86_64-apple-darwin.tar.gz"
      sha256 "f4e10262a50c2d42c8dfe7c42bf083cbfe0e300a087fd5338335fbe2877a4e0b"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.8/illumio-tui-0.5.8-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0c0794a73050787a2e56aafc0fecdccb5e5a0ebe4d4e14e7cc6b88fa54947ed4"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.8/illumio-tui-0.5.8-x86_64-unknown-linux-musl.tar.gz"
      sha256 "f43d8474a12b49681e7bb7e010fc595abd79c02f89cb9c165fdfb9b24b3be498"
    end
  end

  def install
    bin.install "illumio-tui"
    # `illumio-tui completions bash|zsh|fish` prints each script.
    generate_completions_from_executable(bin/"illumio-tui", "completions")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/illumio-tui --version")
    assert_match "__complete profiles", shell_output("#{bin}/illumio-tui completions zsh")
  end
end
