# Third-party source notices

Some crates omit repository-root license files from their crates.io archives. These unmodified notices are kept locally so a fresh native build never fetches licenses from a changing website. Native dependency versions remain locked in Cargo.lock.

- alloc-stdlib 0.3.0: [Dropbox BSD-3-Clause](https://github.com/dropbox/rust-alloc-no-stdlib/blob/master/LICENSE).
- defmt-parser 1.0.0: [MIT](https://github.com/knurling-rs/defmt/blob/main/LICENSE-MIT), [Apache-2.0](https://github.com/knurling-rs/defmt/blob/main/LICENSE-APACHE).
- selectors 0.38.0: [Mozilla Public License 2.0](https://www.mozilla.org/media/MPL/2.0/index.txt), [unmodified crate source](https://crates.io/crates/selectors/0.38.0).
- webview2-com 0.39.1, webview2-com-macros 0.8.1, webview2-com-sys 0.39.1: [MIT](https://github.com/wravery/webview2-rs/blob/main/LICENSE).

Nongfsq attribution and MIT text remain in modules/cambridge-dictionary-api/LICENSE.txt. The generated distribution notice aggregates these with registry notices; Node and the standalone JS API dependencies are not part of the native desktop binary.
