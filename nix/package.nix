{
  lib,
  rustPlatform,
  stdenv,
  clang,
  specCorpusV1,
  specCorpusV2,
  wild ? null,
}:
let
  cargoTOML = (lib.importTOML ../Cargo.toml).workspace.package;

  # wild + clang are only used on Linux tier-1 arches
  hasWild =
    stdenv.hostPlatform.isLinux && (stdenv.hostPlatform.isx86_64 || stdenv.hostPlatform.isAarch64);
in
rustPlatform.buildRustPackage {
  pname = "knead";
  inherit (cargoTOML) version;

  src =
    let
      fs = lib.fileset;
      s = ../.;
    in
    fs.toSource {
      root = s;
      fileset = fs.unions [
        (s + /src)
        (s + /knead-derive)
        (s + /tests/spec.rs)
        (s + /examples)
        (s + /benches)
        (s + /Cargo.lock)
        (s + /Cargo.toml)
      ];
    };

  cargoLock.lockFile = ../Cargo.lock;
  cargoTestFlags = [ "--workspace" ];

  strictDeps = true;
  nativeBuildInputs = lib.optionals hasWild [
    wild
    clang
  ];

  env = {
    KNEAD_SPEC_CORPUS_V1 = specCorpusV1;
    KNEAD_SPEC_CORPUS_V2 = specCorpusV2;
  }
  // lib.optionalAttrs hasWild {
    RUSTFLAGS = "-Clinker=${clang}/bin/clang -Clink-arg=--ld-path=wild";
  };

  enableParallelBuilding = true;

  meta = {
    description = "KDL v2 parser and typed decoder";
    license = lib.licenses.eupl12;
    maintainers = with lib.maintainers; [ amaanq ];
  };
}
