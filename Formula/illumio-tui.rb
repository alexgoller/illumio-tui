# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.5.7"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.7/illumio-tui-0.5.7-aarch64-apple-darwin.tar.gz"
      sha256 "e611609152044f2ca72a25274de96da3c268af51101b812c1a695e51d3ecba4d"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.7/illumio-tui-0.5.7-x86_64-apple-darwin.tar.gz"
      sha256 "cc6d8612466320357010b78d543b227fdcdeb8878b3c187402837b1d2c8530c6"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.7/illumio-tui-0.5.7-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "713473d5c1b84f943308d7777a2f47122f8549870f4225f3fd6b18b3c779a74a"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.7/illumio-tui-0.5.7-x86_64-unknown-linux-musl.tar.gz"
      sha256 "7542c1a0c68597b6fc1c1d2655d8bb19589ad97de0605af0b5cd7d95609539e5"
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
