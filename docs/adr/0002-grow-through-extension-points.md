# ADR-0002: Grow the product through features that plug into extension points

|          |                                                                                                                                                |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D14)                                                                                                                                 |
| Date     | 2026-10-13                                                                                                                                     |
| Deciders | Owner                                                                                                                                          |
| Related  | [Feature protocol](../features/README.md), [Extension points](../architecture/extension-points.md), [Repository](../engineering/repository.md) |

Later changes: [ADR-0034](0034-build-plugins-on-a-plugin-sdk.md) refines decisions 3, 4 and 6: plugins register through the plugin SDK's extension points, a missing point is an SDK change with an ADR, and four IPC shapes of plugins change once in v0.3.0. [ADR-0035](0035-find-plugins-at-build-time-in-folders-of-their-own.md) refines the refinement of M5: a plugin registers as a reviewed `plugin.toml` that the build finds.

## Context

- The owner will ask for features as the need appears, often in rough terms ([decision D14](../roadmap/milestones.md#5-decisions)). There is no feature inventory up front.
- A harness that adds features quickly by patching them into existing code ends up with large files that mix concerns, conditions spread across modules, and changes that break unrelated behavior.
- The owner does not want a heavy specification process with separate specification, plan and task documents for every change.

## Decision

1. Every request is sized first, as the [feature protocol](../features/README.md) describes: a tweak, a feature or an architectural change.
2. A feature has one feature document, or a folder of documents when it is big. No other paperwork is required per feature.
3. A feature's code is a vertical slice that registers with extension points: the tool registry, agent definitions, provider adapters, instruction sections, transcript part types and run events, IPC commands and channels, UI shell slots, settings sections, store migrations and link services. The [extension points](../architecture/extension-points.md) document defines each one.
4. Contracts and stored data change only by addition: new commands, new event variants, new tables and columns, new protobuf fields and services. Readers tolerate variants they do not know. Removing something takes a deprecation period of at least one minor release.
5. A feature uses another feature only through its public interface or an extension point, never through its internals.
6. When a feature cannot be built without editing unrelated modules, an extension point is missing. An ADR adds it in a general form first, and the feature is then built on it.
7. Cross-cutting documents describe extension points and rules. They do not list features.
8. An extension point is created when the first feature needs it, not in advance.

## Consequences

- Positive: a feature's change stays inside its slice, which keeps reviews small and rarely breaks another feature. Documentation changes stay local as well.
- Negative: registries add a layer of indirection, and additive contracts collect deprecated items until they are removed.
- Risks: an extension point designed before a real feature uses it may be wrong. Rule 8 limits this, and a wrong extension point is replaced through an ADR.
- Refinement in M5: features can form plugins that the owner turns on and off while Mannie runs, and the registries take and drop a plugin's registrations ([ADR-0030](0030-split-mannie-into-a-core-and-plugins.md)). Registration stays explicit code in a composition root, and nothing is loaded at runtime.

## Alternatives considered

| Option                                                  | Why not chosen                                                                                           |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| An up-front capability catalog with priorities          | The owner does not want a feature inventory, and a catalog needs an edit for every new idea              |
| Specification, plan and task documents for every change | Too heavy for one owner, and the documents repeat each other                                             |
| A runtime plugin system                                 | Loading code at runtime brings security and compatibility problems that compile-time registration avoids |

## Validation

None needed: the owner decided this (D14). Each milestone's review checks that its features stayed inside their slices.
