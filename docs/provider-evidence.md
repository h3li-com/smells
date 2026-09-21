# Optional deterministic provider evidence

All 28 registered rules have built-in collectors and executable evaluators. A normal
`smells check` therefore needs no evidence file. Teams with higher-fidelity compiler,
coverage, type-resolution, project-contract, complete-test, or history facts may supply a
versioned provider-evidence bundle with `--evidence FILE`. The scanner never accepts a
provider's match verdict: it validates exact measurement keys and locations, then applies
the policy threshold itself with integer cross-products.

The bundle must select the same rule pack and scanner version and must pin the exact
`input_sha256` printed by the same scan input. Evidence used with `--staged` must itself be a
regular staged Git blob. Include only rules whose built-in observations should be replaced;
each included rule needs one unique entry with `complete: true`, even when its observation
list is empty. Omitted rules use built-in collectors. Stale, duplicate, incomplete,
malformed, unknown, or source-owned provider entries fail closed. Provider
name, version, and configuration SHA-256 are retained in each finding; the report also
contains `provider_evidence_sha256` for the complete bundle bytes.

Provider identity and `complete` are explicit trust-boundary assertions, not an
authentication mechanism. Runtime validation checks their structure and binds their exact
bytes to the report; the surrounding hook must invoke the intended pinned producer and
verify that its declared name, version, and configuration digest are the expected ones.
The scanner cannot prove that an external producer truthfully emitted an empty complete
observation set. This boundary is why optional provider execution remains outside the
scanner and why the supplied bundle is retained by digest.

Built-in observations identify provider `smells-built-in` and include a versioned collector
name and scope. External observations take precedence only for the rule they name; they do
not disable or alter other active rules.

Use [the JSON Schema](../schemas/provider-evidence.schema.json) for the envelope. Runtime
validation additionally enforces these exact measurement maps:

| Rule suffix | Measurements |
| --- | --- |
| `primitive_slots` | `raw_slots`, `total_slots` |
| `alternative_interfaces` | `first_tokens`, `second_tokens`, `intersection`, `union` |
| `repeated_dispatch` | `sites`, `arms_per_site` |
| `temporary_fields` | `fields`, `methods`, `uses`, `possible_uses` |
| `forwarding_share` | `methods`, `forwarders` |
| `function_crap` | exact rational `numerator`, `denominator` |
| `unused_code`, `unused_type_parameters` | `findings` |
| `nominal_slot_contract` | `mismatches` |
| `port_conformance`, `library_capabilities` | `failures` |
| `refused_bequest` | `inherited_members`, `unused_members` |
| `divergent_change` | `changes`, `responsibilities` |
| `parallel_inheritance` | `parallel_pairs` |
| `shotgun_surgery` | `owners` |
| `foreign_accesses` | `foreign_accesses`, `total_accesses` |
| `dependency_contract` | `forbidden_accesses` |
| `navigation_chains` | `transitions` |

Locations must resolve inside the captured corpus and related symbol/location arrays must
have equal lengths. Provider-specific trace data belongs in `evidence`; it is reported as
untrusted data and never controls the threshold result.
