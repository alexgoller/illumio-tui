# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.5.4"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.4/illumio-tui-0.5.4-aarch64-apple-darwin.tar.gz"
      sha256 "0d0dcdb4b937498b69ad911a77826b720f291de0facdc722d654b47c8e66c997"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.4/illumio-tui-0.5.4-x86_64-apple-darwin.tar.gz"
      sha256 "c92d293c06f3e0af295125346dab68978f02749b27febfe685eb1a994f5cc8f9"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.4/illumio-tui-0.5.4-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "48e82d2ce1d9b040f6937474155de4cb7eb300fb8f990ce2a44e1840582e4a6a"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.4/illumio-tui-0.5.4-x86_64-unknown-linux-musl.tar.gz"
      sha256 "64a14adc370f8cb5fc16621fec856fd9a87f9949748608e220a87a58435c8e39"
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
