class Qyzs < Formula
  desc "Qingyang AI Computing Center unified CLI for large models and document pipelines"
  homepage "https://github.com/qyzs-ai/qyzs-cli"
  version "0.2.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/qyzs-ai/qyzs-cli/releases/download/v0.2.0/qyzs-aarch64-apple-darwin.tar.gz"
      sha256 "dbddd5a4255f47f95088ffd180b028c73f7d317df9cd2b3601be65381cf2c6f6"
    else
      url "https://github.com/qyzs-ai/qyzs-cli/releases/download/v0.2.0/qyzs-x86_64-apple-darwin.tar.gz"
      sha256 "524556f8fef6ef2f762de68b8bdf73dd556eacd3e1e90712799968e1971f2ca4"
    end
  end

  def install
    bin.install "qyzs"
  end

  test do
    assert_match "qyzs", shell_output("#{bin}/qyzs --version")
  end
end
