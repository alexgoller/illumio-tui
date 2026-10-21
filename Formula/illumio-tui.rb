# Rendered by .github/workflows/release.yml from packaging/homebrew/illumio-tui.rb.tmpl
# on every tagged release. Edit the template, not this file.
class IllumioTui < Formula
  desc "Terminal UI and CLI for the Illumio Core PCE"
  homepage "https://github.com/alexgoller/illumio-tui"
  version "0.5.0"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.0/illumio-tui-0.5.0-aarch64-apple-darwin.tar.gz"
      sha256 "c8b9abf098a85bc0a3cc8251a97b2a39f9a0e77071ec9bf0fd3a3fb8e449f491"
    end
    on_intel do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.0/illumio-tui-0.5.0-x86_64-apple-darwin.tar.gz"
      sha256 "ee4879887bec7d56f4484c9be25e4a0a2be5f42348169daa19c9874a181a6ad4"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.0/illumio-tui-0.5.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "b39ba4334bd397c612fdca6b4f430d3bfb7c9e7b33ec0519b1c03c92ef5c60cd"
    end
    on_intel do
      # The static musl build: no glibc floor, runs on any x86_64 Linux.
      url "https://github.com/alexgoller/illumio-tui/releases/download/v0.5.0/illumio-tui-0.5.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "d0dd2c5f6a9cf3154539b6f6644cc3d104f842402cab85f5eba2d780da911e41"
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
