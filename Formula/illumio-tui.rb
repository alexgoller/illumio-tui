# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.4.2"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.2/illumio-tui-0.4.2-aarch64-apple-darwin.tar.gz"
      sha256 "cf1d7523a740c12e3cfdfbc8478152c7161de15684f49b8c581e7e3469672163"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.2/illumio-tui-0.4.2-x86_64-apple-darwin.tar.gz"
      sha256 "38a99a44db11531289af3a9a0bfcc242dd0b6fa58692e8060c3750470863b20c"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.2/illumio-tui-0.4.2-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "73f045ce708471f6c19e3bb035e220e008c76b1c5bf34d80bd2c0fbc9f732daf"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.2/illumio-tui-0.4.2-x86_64-unknown-linux-musl.tar.gz"
      sha256 "2f598990c427e717bede48b9c59321692f7ff032ca93be5aa6891c0c290f18ef"
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
