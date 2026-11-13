# Components

A component is a cohesive part of the compiled product with one responsibility.
Every file that lands in the executable belongs to exactly one component. The
[schema](schema.json) defines the structure; this document explains why the
artifact exists and how to keep it true.

## Why components exist

The other knowledge artifacts say what the product must do and why it is built
that way. None of them says where anything lives. Without that link an agent has
to discover the structure by reading code every time it starts a task, and a
reviewer cannot tell whether a change belongs where it was made.

Components fill that gap at the coarsest useful level: files grouped by
responsibility, with explicit boundaries. They are deliberately not about
classes, functions or variables, which change constantly and would make the map
stale within days.

## How agents use them

- **Planning.** From a task, find the responsible component, its files and the
  components it depends on, so the affected area is known before any code is read.
- **Implementing.** `scope` says what belongs here, `out_of_scope` says who owns
  the neighboring matter, and `must_not` lists prohibitions that hold even when
  a task seems to call for them. A task that needs something out of scope
  belongs in the owning component.
- **Reviewing.** A change can be checked against the boundaries of every
  component it touches. A prohibition that is violated is a finding even when all
  tests pass.
- **Creating files.** A new compiled file forces a decision about its component,
  which is the moment a misplaced responsibility is cheapest to catch.

## What a component is

- **One responsibility.** The name states what the component is responsible for,
  not what a file is called. Split a component when its scope needs the word
  "and" between unrelated matters. Merge components when neither can be
  described without the other.
- **Files, exactly.** `files` lists every file by its path from the repository
  root. There are no wildcards and no directories, so a new file cannot join a
  component unnoticed. A file belongs to exactly one component; if it cannot be
  assigned to one, the file mixes responsibilities and should be split.
- **The compiled product only.** The partition covers every file below `src`
  and every file the sources embed at compile time. Tests, build scripts and
  release scripts are not components. They are verified through requirements.
- **A view of the implementation.** Components describe the current structure.
  They are not specification, so they hold no requirements text. A component
  names the requirements it implements and the decisions that constrain it;
  requirements and decisions never name components, so the specification
  survives refactoring.

## Fields

| Field | What to write |
| --- | --- |
| `purpose` | One sentence on why the component exists. |
| `scope` | Responsibilities, each a complete sentence about one matter. |
| `out_of_scope` | Matters a reader might expect here that belong elsewhere. Name the `owner` component when one exists; omit it when nobody owns the matter. |
| `must_not` | Prohibitions for the code of this component. A prohibition differs from a boundary: `out_of_scope` says whose job something is, `must_not` says what is forbidden. |
| `files` | Every file of the component, one path each. |
| `depends_on` | Components whose code this one uses. It must equal the dependencies in the source, so it is a record, not a wish. |
| `decisions`, `requirements` | Constraints and obligations that apply to the whole component. Link only what shapes it; a decision that touches every component belongs on none. |

Keep statements short and testable by reading code. A prohibition no reviewer
could ever see violated does not belong here.
[COMP-0007](COMP-0007.yaml) is a representative example.

## How it stays current

The `knowledge` rule in [`.please/checks/knowledge`](../../.please/checks/knowledge/check.py)
runs at pre-commit, pre-push and in CI. It fails when

- a component file does not follow the schema, or a reference does not resolve,
- a compiled file belongs to no component or to more than one,
- a listed file is not part of the compiled product, or
- `depends_on` differs from the dependencies found in the source files.

It also reports components that depend on each other. Those are not failures;
they show where the structure is entangled and are candidates for refactoring.

When a change adds, moves or removes a compiled file, or adds a dependency
between components, update the component in the same commit; the check enforces
it. When a responsibility moves, update `scope`, `out_of_scope` and `must_not`
of both components. When a file cannot be assigned to one component, split the
file instead of weakening the rule.

## What the check cannot see

The check proves that the map is complete and that the dependencies are real. It
cannot prove that `scope` and `must_not` are still true, because they are
statements about intent. They stay reliable through review: a change that
touches a component's files is read against that component's boundaries, and a
periodic independent review looks for drift, gaps and overlaps between
components.
