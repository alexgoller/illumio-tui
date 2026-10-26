# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.5.6"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.6/illumio-tui-0.5.6-aarch64-apple-darwin.tar.gz"
      sha256 "82ed493773d829233c9b490eb9b7cd0c5a46e341bd3172cd44994916331c3d65"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.6/illumio-tui-0.5.6-x86_64-apple-darwin.tar.gz"
      sha256 "666df8a53db05cc50bb5388caf64199f8ce06d3a1f7440a28b3f3eb834731b39"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.6/illumio-tui-0.5.6-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "8f4cffb5230c54aefa47cb57ced0f01eec417a4179cf4565c29db3d69b0c2f4c"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.6/illumio-tui-0.5.6-x86_64-unknown-linux-musl.tar.gz"
      sha256 "0e43048b0cdc452675b7e8c1d22cc7f517644f7a0432a7b6f7aca70a9f9bc0d2"
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
