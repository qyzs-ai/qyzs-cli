class Qyzs < Formula
  desc "Qingyang AI Computing Center unified CLI for large models and document pipelines"
  homepage "https://github.com/qyzs-ai/qyzs-cli"
  version "0.2.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/qyzs-ai/qyzs-cli/releases/download/v0.2.0/qyzs-aarch64-apple-darwin.tar.gz"
      sha256 "9718adcbc6cedc728143ebed78ebd6d00cfa5e139d006fd296c731ede266b8e8"
    else
      url "https://github.com/qyzs-ai/qyzs-cli/releases/download/v0.2.0/qyzs-x86_64-apple-darwin.tar.gz"
      sha256 "0529b684b440bfc55104b711fa7fb71bc12f6ade45ac8ee5ab37e7a2bdd125c3"
    end
  end

  def install
    bin.install "qyzs"
  end

  test do
    assert_match "qyzs", shell_output("#{bin}/qyzs --version")
  end
end
