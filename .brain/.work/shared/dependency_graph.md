# Dependency Graph

```text
WS01 formats ──┬── WS02 node ──┬── WS05 Android
               │                ├── WS06 clients
               └── WS04 repair ─┘
WS03 control plane ─────────────┬── WS02 enrollment
                                ├── WS04 credits/audit
                                └── WS06 tenancy
WS07 infrastructure supports all workstreams
QA01/QA02 validate every merge gate
```

The first shared contracts are shard format, manifest schema, peer protocol, invitation schema, event schema, and status schema.
