{ lib, rustPlatform, cacert }:

rustPlatform.buildRustPackage {
  pname = "jev-bot";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./src
    ];
  };

  cargoLock.lockFile = ./Cargo.lock;

  # The tests only ever talk to loopback, but reqwest's client constructor
  # loads the system trust store up front and fails without one.
  nativeCheckInputs = [ cacert ];
  preCheck = ''
    export SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt
  '';

  meta = {
    description = "Discord bot that scores how likely a statement is to be true, using TypeSafe's jev model";
    homepage = "https://github.com/parzivale/jev-bot";
    mainProgram = "jev-bot";
    platforms = lib.platforms.unix;
  };
}
