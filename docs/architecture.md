# Hexagonal architecture

Smells uses Cockburn's inside/outside boundary directly. In Cockburn's terminology,
the **application** is the whole inside of the hexagon; it is not an additional layer
or directory. This repository calls that inside `core`.

The direction names describe control:

- a **driving** (primary) adapter is inbound and invokes the core through an inbound
  port;
- a **driven** (secondary) adapter is outbound and is invoked by the core through an
  outbound port; and
- port interfaces belong to the core. Adapters depend on those interfaces, never the
  reverse.

The target structure is:

```text
src/
├── core/                              # the inside/application
│   ├── model/                         # domain values and captured input
│   ├── migration.rs                   # pure migration classification policy
│   ├── scanner.rs                     # scan use case and orchestration
│   └── ports/
│       ├── inbound/scanner.rs         # what driving actors may ask Smells to do
│       └── outbound/repository.rs     # repository state the core needs
└── adapters/
    ├── inbound/cli.rs                 # driving CLI adapter
    └── outbound/repository/mod.rs     # driven filesystem/Git adapter
```

There is deliberately no `application/` directory. Use-case orchestration belongs in
`core` unless the codebase later reveals a real, independently useful boundary. There
is also no port for a service that has only one concrete implementation and no
boundary-testing need.

Some analysis modules still live directly under `src/` while this refactor proceeds
in behavior-preserving slices. They are internal implementation called by `core`; new
external technology dependencies must enter through an adapter instead of expanding
that transitional layout.

This follows the original pattern's inside/outside asymmetry and its primary/driving
versus secondary/driven terminology:

- [Alistair Cockburn's original Hexagonal Architecture article](https://alistair.cockburn.us/hexagonal-architecture)
- [AWS Prescriptive Guidance: Hexagonal architecture pattern](https://docs.aws.amazon.com/prescriptive-guidance/latest/cloud-design-patterns/hexagonal-architecture.html)
