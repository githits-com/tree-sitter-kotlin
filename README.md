# tree-sitter-kotlin

[![CI][ci]](https://github.com/tree-sitter-grammars/tree-sitter-kotlin/actions/workflows/ci.yml)
[![discord][discord]](https://discord.gg/w7nTvsVJhm)
[![matrix][matrix]](https://matrix.to/#/#tree-sitter-chat:matrix.org)
[![crates][crates]](https://crates.io/crates/tree-sitter-kotlin-ng)
[![npm][npm]](https://www.npmjs.com/package/@tree-sitter-grammars/tree-sitter-kotlin)
[![pypi][pypi]](https://pypi.org/project/tree-sitter-kotlin)

Kotlin grammar for [tree-sitter](https://github.com/tree-sitter/tree-sitter).

## References

- [Kotlin Grammar](https://kotlinlang.org/docs/reference/grammar.html)

[ci]: https://img.shields.io/github/actions/workflow/status/tree-sitter-grammars/tree-sitter-kotlin/ci.yml?logo=github&label=CI
[discord]: https://img.shields.io/discord/1063097320771698699?logo=discord&label=discord
[matrix]: https://img.shields.io/matrix/tree-sitter-chat%3Amatrix.org?logo=matrix&label=matrix
[npm]: https://img.shields.io/npm/v/@tree-sitter-grammars/tree-sitter-kotlin?logo=npm
[crates]: https://img.shields.io/crates/v/tree-sitter-kotlin-ng?logo=rust
[pypi]: https://img.shields.io/pypi/v/tree-sitter-kotlin?logo=pypi&logoColor=ffd242

## Maintained aigrep backport

This GitHits fork keeps package `tree-sitter-kotlin-ng` at 1.1.0, based on its
published commit `77dd60ea0a9003ce062c9728a513ffe1aaff8c82`. Aigrep pins the full
fork commit; the backport branch is retained without deletion or force-push while
any consumer pins it. No fork crate is published or tagged.

The only compiled-source change is adding an EOF condition to two annotation
lookahead loops in `src/scanner.c` (constructor and GET/SET lookahead). Credit:
[mrmans0n's upstream PR #18](https://github.com/tree-sitter-grammars/tree-sitter-kotlin/pull/18),
commit `821f604ad6da6e41a871749fa53cde71031cef78`. Generated parser, grammar,
headers, Rust bindings, package version and dependencies remain unchanged; this
README is the only other changed file. The upstream MIT license and attribution
are retained.

Without these conditions, annotation lookahead can advance forever at EOF,
holding a parse worker and causing production JetBrains Kotlin indexing timeouts.
The activating input is exactly 60 bytes with no final newline:

```kotlin
// FILE: script.kts

annotation class Ann

@Ann<!SYNTAX!><!>
```

Close this fork when a published upstream release contains the scanner repair
and passes the same EOF reproducer plus byte-identical aigrep graph/chunk/flow
output for the previously successful existing Kotlin fixtures and registered
Moshi corpus. Switch aigrep back to that exact registry release, remove its Git
source exception, then archive this fork after verifying no consumer still pins
it. A merged upstream PR alone is insufficient; the published package must
contain the qualified fix.
