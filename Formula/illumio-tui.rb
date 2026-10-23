# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.5.3"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.3/illumio-tui-0.5.3-aarch64-apple-darwin.tar.gz"
      sha256 "94480a7948b9aefed252e9f6b56402cd4473619f2fcde1b324c951050c2e7e6c"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.3/illumio-tui-0.5.3-x86_64-apple-darwin.tar.gz"
      sha256 "2d1aa0e7a2333bbcdc70f0240f3c757e8849f05217b8defc51a92606f03442c3"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.3/illumio-tui-0.5.3-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "1a7e366f464e09c7919f62b016083d1502d758e61f0808523421c49cb7627602"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.3/illumio-tui-0.5.3-x86_64-unknown-linux-musl.tar.gz"
      sha256 "a41c4d30d1586f05ef183176535da9bc8f1f7350a6e6660311554d95e90dd250"
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
