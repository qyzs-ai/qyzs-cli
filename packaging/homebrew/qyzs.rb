class Qyzs < Formula
  desc "Qingyang AI Computing Center unified CLI for large models and document pipelines"
  homepage "https://github.com/qyzs-ai/qyzs-cli"
  version "0.2.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/qyzs-ai/qyzs-cli/releases/download/v0.2.0/qyzs-aarch64-apple-darwin.tar.gz"
      sha256 "ae2c7d8b33cd1b2dc8854be741557d5eb1aa21d4a90317d3391c5a5d9211acc6"
    else
      url "https://github.com/qyzs-ai/qyzs-cli/releases/download/v0.2.0/qyzs-x86_64-apple-darwin.tar.gz"
      sha256 "7e3a5655e1e535531a9ca661b5256b08cbda7397f6a9777486436abff3ae5237"
    end
  end

  def install
    bin.install "qyzs"
  end

  test do
    assert_match "qyzs", shell_output("#{bin}/qyzs --version")
  end
end
