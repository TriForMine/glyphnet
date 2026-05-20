# Security Policy

GlyphNet parses untrusted images and untrusted binary payloads. Parser,
decoder, and scanner bugs should be treated seriously.

## Reporting

Please report suspected vulnerabilities privately through the repository security
advisory feature. If that is unavailable, contact the maintainers listed in the
repository metadata.

## Scope

Security-sensitive areas include:

- binary frame parsing;
- image sampling and matrix decoding;
- future camera backends;
- WebAssembly boundaries;
- mobile FFI boundaries;
- dependencies used in parsing, image loading, and CLI tooling.

## Expectations

- Fuzz regressions should be minimized and committed as fixtures.
- Panics on malformed input are bugs.
- Memory unsafety is not accepted in default builds.
- Release workflows run dependency audits and license checks.
