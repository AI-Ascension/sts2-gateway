# Game-information content manifest v1

This closed, bounded whole-manifest transport carries the exact typed output of
`ContentManifestProducer`, excluding semantic inputs and localized text. The protocol codec
validates one complete UTF-8 JSON envelope and refuses serialized messages larger than 16 MiB; it
does not page or truncate. The #83 game-mod producer adapter has not yet been integrated and must
use this codec before emission.

Query-v1 `content_manifest_id` names the canonical `inventory_revision` only after the owner has
produced this manifest. Error reasons are closed safe tokens: producers must never place semantic
inputs, localized text, host diagnostics, or other free-form content in the error envelope.
