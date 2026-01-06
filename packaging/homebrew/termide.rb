class Termide < Formula
  desc "Cross-platform terminal IDE, file manager and virtual terminal"
  homepage "https://github.com/termide/termide"
  version "0.8.3"
  license "MIT"

  on_macos do
    on_intel do
      url "https://github.com/termide/termide/releases/download/0.8.3/termide-0.8.3-x86_64-apple-darwin.tar.gz"
      sha256 "f51e91b992bf5bfa8165c580e1787236ede625033a72782dcf5765f202318e41"
    end

    on_arm do
      url "https://github.com/termide/termide/releases/download/0.8.3/termide-0.8.3-aarch64-apple-darwin.tar.gz"
      sha256 "696aaeed8f977844174e4ec8ad7c2ade9565afb697543fce12f2f96617657f0d"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/termide/termide/releases/download/0.8.3/termide-0.8.3-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "01caf25f6b943a7a162c2c174c7ec60bece8da6e4e178489223ff8a6458fad79"
    end

    on_arm do
      url "https://github.com/termide/termide/releases/download/0.8.3/termide-0.8.3-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "dfad8823568325fe0ecc1a0280c8c6b42bb2df087f9e1c025394780da23986f8"
    end
  end

  def install
    bin.install "termide"

    # Install help files
    (share/"termide/help").install "help/en.txt"
    (share/"termide/help").install "help/ru.txt"

    # Install documentation
    doc.install "README.md"
    doc.install "LICENSE"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/termide --version")
  end
end
