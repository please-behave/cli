# Requirements

A requirement is one binding, verifiable statement about what Please Behave
must do or be. The [schema](schema.json) defines the structure, including the
areas; this document explains why the artifact exists and how to keep it true.

## Why requirements exist

Requirements are the contract between what the product is meant to be and what
is built. Components say where things live, decisions say why a significant
choice was made, but only requirements say what must hold. Everything else is
measured against them: an implementation that violates a requirement is wrong,
and a requirement that nothing verifies is a claim.

They are written for the person who owns the product as much as for agents. The
owner reads requirements in plain product language and does not need to read
code to know what the product promises.

## How agents use them

- **Planning.** A task is mapped to the requirements it touches. A task that
  changes behavior starts by changing a requirement, then the tests, then the
  code.
- **Implementing.** The requirement states the observable condition to satisfy,
  and the tests that cite it show how it is verified.
- **Reviewing.** A change is judged against the requirements of the components
  it touches. A violated requirement is a finding even when no test caught it.
- **Finding gaps.** A requirement no test cites is unverified, and behavior that
  no requirement describes is unspecified. Both are visible and neither is
  acceptable for long.

## What a requirement is

- **One statement.** A requirement states one condition that can be verified on
  its own. If its description needs "and" between unrelated conditions, it is
  several requirements.
- **Observable.** It describes behavior or a property someone can see from
  outside, not a function, a type or a file. How it is achieved belongs to
  components and decisions.
- **Binding and verifiable.** A reader can say how to find out whether it holds.
  Wording such as "must" marks it as binding; vague wording such as "fast" or
  "robust" is not a requirement.
- **Stated once.** A fact that another source already defines, such as a default,
  a limit or an exit code in the [configuration reference](../../docs/configuration.md),
  is referred to, not copied, so the two cannot drift apart.
- **Significant.** A requirement is worth keeping when it is durable, so that it
  stays true after a refactoring, visible from outside the product, and
  justified in one sentence. A statement that follows from other requirements,
  or whose later change nobody would notice, is merged into another or dropped.
- **Code-free.** A requirement never names components, tests or source files.
  Components name the requirements they implement and tests name the ones they
  verify, so the specification survives refactoring.

An example of the right size: "A check must start only after every check it
depends on has completed with PASS." A statement that bundles scheduling,
dependency validation and reporting is too large.

## Writing them well

- **State the condition.** A trigger belongs in the statement: "When fail-fast is
  requested, ..." rather than a bare "must trigger".
- **Name the obligation precisely.** Write "must not" or "may not" for
  prohibitions. A negated "must be invoked" can be read as "nobody has to".
- **Say it once.** A property that holds for several settings, such as an
  unchanged plan identity, is one requirement and not one per setting. A
  statement that follows from another one is dropped.
- **Keep the original meaning.** Do not tighten or loosen the conditions of the
  source text. Check quantifiers such as "outside quotes" against "outside
  single quotes".
- **Leave out what is not a promise.** Hints for rule authors, known limitations,
  internal file or field names and implementation steps belong in the
  documentation.
- **Refer instead of copying.** Defaults, bounds and character lists stay in the
  configuration reference.
- **Mention no earlier forms.** Before 1.0.0 there are no deprecations, so a
  requirement describes only what exists. Rejecting a retired form is not a
  requirement.
- **Cite a decision sparingly.** Only when a significant choice with a real
  alternative stands behind the requirement.
- **Do not resolve conflicts silently.** When the source text contradicts a
  decision or the product vision, ask the owner and record where the code
  deviates in a GitHub issue.

## Fields

| Field | What to write |
| --- | --- |
| `name` | A short noun phrase that identifies the requirement. |
| `area` | Exactly one of the areas listed in the schema. The overview of what the product can do is generated from the areas, so it is never written by hand. |
| `description` | The statement itself. |
| `rationale` | One plain sentence on why the product needs this. It is what the owner judges, since the statement alone says what holds but not why anyone wants it. If the reason needs more than a sentence, the requirement is probably too large or too small. |
| `decisions` | The decisions that shaped this requirement. Omit it when no significant choice stands behind it; most requirements need none. |
| `obsolete` | Set to `true` when the requirement no longer binds. Delete a requirement that was withdrawn without replacement. |

References point from this artifact to the more stable one: a requirement names
the decisions that shaped it, and a decision never names requirements. The
reverse links are derived, never written by hand.

## Decisions and requirements

Write the requirement first. Ask whether a significant choice with a real
alternative stands behind it, one whose later change would matter. If so, the
choice is a decision and the requirement cites it. If not, the requirement needs
no decision. The need that triggered a decision stays in its `context`.

## How it stays current

The `knowledge` rule in [`.please/checks/knowledge`](../../.please/checks/knowledge/check.py)
fails when a requirement does not follow the schema, uses an unknown area, or
cites a decision that does not exist. Component references to requirements must
resolve as well.

A change in behavior starts with the requirement. Update it in the same commit
as the tests and the code. A requirement that no longer holds is marked obsolete
or deleted, never left in place and ignored.

## What the check cannot see

The check proves that requirements are well-formed and that references resolve.
It cannot prove that a requirement is correct, complete or non-overlapping.
Those properties stay reliable through the owner's reading of new and changed
requirements and a periodic independent review for gaps and overlaps.
