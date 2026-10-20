# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.4.0"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.0/illumio-tui-0.4.0-aarch64-apple-darwin.tar.gz"
      sha256 "b71337920794d830d1ebae0493c15c65c45e94483eab9a67eae6f5e7a110f8a9"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.0/illumio-tui-0.4.0-x86_64-apple-darwin.tar.gz"
      sha256 "da2e8b20357875eed1f97d0b0073b61fcafa26011754bfb84ef6bf95d642d6c5"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.0/illumio-tui-0.4.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "d4fa2d1a9e0672d654e523ad8daddb934c3c1529ddd4af002aae7e7cdffe8179"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.4.0/illumio-tui-0.4.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "4fd9deb2a2aabb7159f46a7b1e1a42c053cf3ca9adc7d57261c09c507909b5b1"
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
