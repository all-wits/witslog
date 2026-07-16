class Witslog < Formula
  desc "AI-native, SQLite-backed error intelligence framework"
  homepage "https://github.com/all-wits/witslog"
  version "0.1.0"

  on_macos do
    on_arm do
      url "https://github.com/all-wits/witslog/releases/download/v0.1.0/witslog-macos-aarch64.tar.gz"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
    on_intel do
      url "https://github.com/all-wits/witslog/releases/download/v0.1.0/witslog-macos-x86_64.tar.gz"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/all-wits/witslog/releases/download/v0.1.0/witslog-linux-aarch64.tar.gz"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
    on_intel do
      url "https://github.com/all-wits/witslog/releases/download/v0.1.0/witslog-linux-x86_64.tar.gz"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
  end

  def install
    bin.install "witslog"
  end

  test do
    system "#{bin}/witslog", "--version"
  end
end
