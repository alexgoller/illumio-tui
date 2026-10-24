# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.5.5"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.5/illumio-tui-0.5.5-aarch64-apple-darwin.tar.gz"
      sha256 "dd88bf9a7c6c1fc0a3f42c2c22676872c8eb6d8f6f7e438c2ac0f62884f44d73"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.5/illumio-tui-0.5.5-x86_64-apple-darwin.tar.gz"
      sha256 "a5911ad2101d30db02f10d3597ea2477c0918c8235985c9accf8dc14810f1e59"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.5/illumio-tui-0.5.5-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "744f85d6acf88f4d453853745dc01326b7e956dc9b0c79f084dabfbac2dd87d7"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.5/illumio-tui-0.5.5-x86_64-unknown-linux-musl.tar.gz"
      sha256 "efe3c9b554fa711898739041ebcb535eb985fc130fb932a58ace84a687090e5c"
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
