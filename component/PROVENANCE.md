# Interface provenance and verification boundary

`wit/niles-integration.wit` is copied byte-for-byte from
https://github.com/MarkNygaard/niles/blob/2511026a01a111b0dcdf1287b54ce372f4555bc6/schemas/niles-integration.wit
(PR #443 implementation, interface 0.3.0).

SHA-256: `433d899f376ec4e061646c3ca0214c379ae70e7582ab4a08afc9f5567b27e5ec`.

Production imports are generated from this file. `src/test_host.rs` supplies explicitly synthetic imports under `cfg(test)` using the generated WIT types; tests call the same component entry points as production. These tests do not establish host encryption durability, database persistence, host permission behavior, public callback delivery, or actual provider compatibility.

Manual `user_id`, `device_id`, room and OAuth client settings remain required. Setup UI is unfinished. Provider calls are implemented, but no real provider/service operations were performed during this migration. Do not deploy until actual Niles 0.3 loading and host storage semantics are verified. In particular a host in-memory secrets fallback is not durable credential persistence; this component does not have a WIT capability to detect that fallback.
