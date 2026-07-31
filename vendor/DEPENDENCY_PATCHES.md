# Vendored Dependency Replacements

These two small crates are vendored only to replace unmaintained transitive
packages without changing the public dependency names expected by upstream
Longbridge, Parquet, and Statrs releases.

| Compatibility name | Maintained source | Pinned source |
| --- | --- | --- |
| `dotenv` | [dotenvy](https://github.com/allan2/dotenvy) | `0.15.7`, commit `fa25166994d6978bd2e002f0ed190c0c39674ebe` |
| `paste` | [pastey](https://github.com/as1100k/pastey) | `0.2.3`, commit `4f134ba98ddd307ff7920d3701eed9522976a1b8` |

The original upstream licenses are retained in each vendor directory. The
crate package and library names are adapted locally so Cargo can substitute
them for the exact transitive package names while preserving source APIs.
