# Interface provenance and verification boundary

`wit/niles-integration.wit` is copied byte-for-byte from
https://github.com/MarkNygaard/niles/blob/2511026a01a111b0dcdf1287b54ce372f4555bc6/schemas/niles-integration.wit
(PR #443 implementation, interface 0.3.0).

SHA-256: `433d899f376ec4e061646c3ca0214c379ae70e7582ab4a08afc9f5567b27e5ec`.

Production imports are generated from this file. `src/test_host.rs` supplies explicitly synthetic imports under `cfg(test)` using the generated WIT types; tests call the same component entry points as production. These tests do not establish host encryption durability, database persistence, host permission behavior, public callback delivery, or actual provider compatibility.

`user_id` is optional: OAuth establishes identity and bounded encrypted account records restore it on restart. An explicitly configured value is only an account restriction and malformed values fail closed. `device_id` remains optional; mat selection is interactive. Room and OAuth client settings remain required. Synthetic component entrypoint tests exercise callback, restore, selection and delivery. No live provider/service operations are claimed.
