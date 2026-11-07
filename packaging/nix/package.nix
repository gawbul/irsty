# Irsty for Nix, built from a release tag. It is written as a nixpkgs
# package (pkgs/by-name/ir/irsty/package.nix), ready to submit. The flake in
# the repository root reuses it with the checked-out source instead, so the
# hashes here only matter for nixpkgs: for a new release, set version, then
# both hashes (set each to lib.fakeHash, build, and copy what Nix reports).
{
  lib,
  rustPlatform,
  fetchFromGitHub,
  installShellFiles,
  versionCheckHook,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "irsty";
  version = "0.1.0";

  src = fetchFromGitHub {
    owner = "gawbul";
    repo = "irsty";
    tag = "v${finalAttrs.version}";
    hash = "sha256-S5sia09hvC5WjFJqAYEW/ivuOFSe8oOzKQjBpDN0NEM=";
  };

  cargoHash = "sha256-FSNL9q9NrSXPNAu8gtac4exFx+HKC44q/5AecN1SiBE=";

  cargoBuildFlags = [ "--package=irsty" ];
  # The full test suite starts mock servers on localhost, which the build
  # sandbox forbids; the protocol tests need nothing.
  cargoTestFlags = [ "--package=irsty-proto" ];

  nativeBuildInputs = [ installShellFiles ];

  postInstall = ''
    installManPage assets/man/irsty.1
    installShellCompletion --cmd irsty \
      --bash assets/completions/irsty.bash \
      --zsh assets/completions/_irsty \
      --fish assets/completions/irsty.fish
  '';

  nativeInstallCheckInputs = [ versionCheckHook ];
  doInstallCheck = true;

  meta = {
    description = "Modern, keyboard-driven terminal IRC client";
    homepage = "https://gawbul.github.io/irsty/";
    changelog = "https://github.com/gawbul/irsty/blob/v${finalAttrs.version}/CHANGELOG.md";
    license = lib.licenses.asl20;
    mainProgram = "irsty";
    platforms = lib.platforms.unix;
  };
})
