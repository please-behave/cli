#!/usr/bin/env python3
"""Check that components and requirements are valid and that the component map matches the code."""

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
LIBRARY = "please"


def load_yaml(path):
    result = subprocess.run(["yq", "-o=json", "-I=0", ".", str(path)], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"yq failed for {path}: {result.stderr.strip()}")
    return json.loads(result.stdout)


def validate(schema, value, where="$"):
    """Validate the JSON Schema subset used by the knowledge schemas and return the violations."""
    errors = []
    kind = schema.get("type")
    checks = {
        "string": lambda v: isinstance(v, str),
        "array": lambda v: isinstance(v, list),
        "object": lambda v: isinstance(v, dict),
        "boolean": lambda v: isinstance(v, bool),
    }
    if kind and not checks[kind](value):
        return [f"{where}: expected {kind}"]
    if "const" in schema and value != schema["const"]:
        errors.append(f"{where}: must equal {schema['const']!r}")
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{where}: {value!r} is not one of {schema['enum']}")
    if kind == "string":
        if len(value) < schema.get("minLength", 0):
            errors.append(f"{where}: must not be empty")
        if "pattern" in schema and not re.search(schema["pattern"], value):
            errors.append(f"{where}: {value!r} does not match {schema['pattern']}")
    if kind == "array":
        if len(value) < schema.get("minItems", 0):
            errors.append(f"{where}: needs at least {schema['minItems']} entries")
        if schema.get("uniqueItems") and len({json.dumps(v, sort_keys=True) for v in value}) != len(value):
            errors.append(f"{where}: entries must be unique")
        for index, item in enumerate(value):
            errors += validate(schema.get("items", {}), item, f"{where}[{index}]")
    if kind == "object":
        properties = schema.get("properties", {})
        errors += [f"{where}: missing {name}" for name in schema.get("required", []) if name not in value]
        if schema.get("additionalProperties") is False:
            errors += [f"{where}: unknown property {name}" for name in value if name not in properties]
        for name, item in value.items():
            if name in properties:
                errors += validate(properties[name], item, f"{where}.{name}")
    return errors


def load_artifacts(root, folder, prefix):
    """Load every `<prefix>-NNNN.yaml` file of a knowledge folder and validate it against the folder schema."""
    directory = root / "knowledge" / folder
    if not directory.is_dir():
        return {}, []
    schema = json.loads((directory / "schema.json").read_text())
    artifacts, errors = {}, []
    for path in sorted(directory.glob(f"{prefix}-*.yaml")):
        artifact = load_yaml(path)
        problems = validate(schema, artifact, path.name)
        if not problems and artifact["id"] != path.stem:
            problems = [f"{path.name}: id {artifact['id']} does not match the file name"]
        errors += problems
        if not problems:
            artifacts[artifact["id"]] = artifact
    return artifacts, errors


def compiled_files(root):
    """Every file below src plus every file the sources embed at compile time."""
    files = {str(path.relative_to(root)) for path in (root / "src").rglob("*") if path.is_file()}
    for path in sorted((root / "src").rglob("*.rs")):
        for target in re.findall(r'include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)', path.read_text()):
            embedded = (path.parent / target).resolve()
            assert embedded.is_file(), f"{path.relative_to(root)}: embeds missing file {target}"
            files.add(str(embedded.relative_to(root)))
    return files


def module_of(path):
    parts = Path(path).relative_to("src").with_suffix("").parts
    return parts[0]


def root_reexports(root):
    """Map each item that lib.rs re-exports to the module that defines it."""
    items = {}
    for module, names in re.findall(r"pub use (\w+)::(\{[^}]*\}|\w+);", (root / "src/lib.rs").read_text()):
        for name in re.findall(r"\w+", names):
            items[name] = module
    return items


def references(text, prefix):
    names = set(re.findall(rf"\b{prefix}::(\w+)", text))
    for group in re.findall(rf"use {prefix}::\{{(.*?)\}};", text, re.S):
        flat = re.sub(r"\{[^{}]*\}", "", group)
        names |= {re.split(r"::|\s", part.strip())[0] for part in flat.split(",") if part.strip()}
    return names


def observed_dependencies(root):
    """File-level edges (source file, target module) from non-test code in src."""
    reexports = root_reexports(root)
    modules = {module_of(str(p.relative_to(root))) for p in (root / "src").rglob("*.rs")}
    edges = set()
    for path in sorted((root / "src").rglob("*.rs")):
        relative = str(path.relative_to(root))
        text = re.split(r"^#\[cfg\([^\]]*\btest\b", path.read_text(), maxsplit=1, flags=re.M)[0]
        names = references(text, "crate")
        if path.name == "main.rs" and path.parent.name == "src":
            names |= references(text, LIBRARY)
        for name in names:
            target = reexports.get(name, name)
            if target in modules and target != module_of(relative):
                edges.add((relative, target))
    return edges


def mutual_pairs(graph):
    """Pairs of components that depend on each other."""
    return sorted((a, b) for a in graph for b in graph[a] if a < b and a in graph.get(b, ()))


def check(root):
    root = root.resolve()
    components, errors = load_artifacts(root, "components", "COMP")
    requirements, requirement_errors = load_artifacts(root, "requirements", "REQ")
    errors += requirement_errors
    errors += load_artifacts(root, "decisions", "DEC")[1]
    for identifier, requirement in requirements.items():
        for reference in requirement.get("decisions", []):
            if not (root / f"knowledge/decisions/{reference}.yaml").is_file():
                errors.append(f"{identifier}: unknown decision {reference}")
    owner = {}
    for identifier, component in components.items():
        for path in component["files"]:
            if path in owner:
                errors.append(f"{path}: assigned to both {owner[path]} and {identifier}")
            owner[path] = identifier
            if not (root / path).is_file():
                errors.append(f"{identifier}: file does not exist: {path}")
    compiled = compiled_files(root)
    errors += [f"{path}: compiled file belongs to no component" for path in sorted(compiled - set(owner))]
    errors += [f"{path}: assigned to {owner[path]} but not part of the compiled product" for path in sorted(set(owner) - compiled)]
    for identifier, component in components.items():
        for reference in component["depends_on"]:
            if reference not in components:
                errors.append(f"{identifier}: unknown dependency {reference}")
            elif reference == identifier:
                errors.append(f"{identifier}: depends on itself")
        for entry in component["out_of_scope"]:
            if entry.get("owner") not in (None, *components):
                errors.append(f"{identifier}: unknown owner {entry['owner']}")
        for reference in component.get("requirements", []):
            if reference not in requirements:
                errors.append(f"{identifier}: unknown requirement {reference}")
    module_owner = {module_of(path): owner[path] for path in owner if path.startswith("src/") and path.endswith(".rs")}
    found = {identifier: {} for identifier in components}
    for source, target in sorted(observed_dependencies(root)):
        origin, destination = owner.get(source), module_owner.get(target)
        if origin and destination and origin != destination:
            found[origin].setdefault(destination, []).append(f"{Path(source).name} uses {target}")
    for identifier, component in components.items():
        declared = set(component["depends_on"])
        for missing in sorted(set(found[identifier]) - declared):
            errors.append(f"{identifier}: undeclared dependency on {missing} ({found[identifier][missing][0]})")
        for stale in sorted(declared - set(found[identifier])):
            errors.append(f"{identifier}: declared dependency on {stale} is not used by any source file")
    notes = [" <-> ".join(pair) for pair in mutual_pairs({k: set(v) for k, v in found.items()})]
    return components, requirements, errors, notes


def main():
    components, requirements, errors, notes = check(ROOT)
    assert not errors, "\n".join(errors)
    for note in notes:
        print(f"NOTE: mutual dependency {note}")
    print(
        f"Knowledge passed: {len(components)} components cover every compiled file exactly once; {len(requirements)} requirements checked."
    )


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:  # A verdict about the knowledge.
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
    except Exception as error:  # A crash is a technical failure, never a FAIL verdict.
        print(f"ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(2)
