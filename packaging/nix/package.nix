# A Nix derivation that builds Irsty from a release tag. Template: set
# version, then the two hashes (build once with lib.fakeHash and copy the
# hashes Nix reports). Use it with callPackage, for example
#   irsty = pkgs.callPackage ./package.nix { };
{
  lib,
  rustPlatform,
  fetchFromGitHub,
  installShellFiles,
}:

rustPlatform.buildRustPackage rec {
  pname = "irsty";
  version = "0.1.0";

  src = fetchFromGitHub {
    owner = "gawbul";
    repo = "irsty";
    rev = "v${version}";
    hash = lib.fakeHash;
  };

  cargoHash = lib.fakeHash;
  cargoBuildFlags = [ "-p" "irsty" ];
  # The full test suite starts mock servers; the protocol tests need nothing.
  cargoTestFlags = [ "-p" "irsty-proto" ];

  nativeBuildInputs = [ installShellFiles ];

  postInstall = ''
    installManPage assets/man/irsty.1
    installShellCompletion --cmd irsty \
      --bash assets/completions/irsty.bash \
      --zsh assets/completions/_irsty \
      --fish assets/completions/irsty.fish
  '';

  meta = {
    description = "Modern, keyboard-driven terminal IRC client";
    homepage = "https://github.com/gawbul/irsty";
    license = lib.licenses.asl20;
    mainProgram = "irsty";
    platforms = lib.platforms.unix;
  };
}
