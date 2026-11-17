# Adapter response

Producer: the external adapter. Consumer: Please Behave.
[response.schema.json](response.schema.json) defines the complete JSON response at
exit status 0 or 1. [Invocation](invocation.md) defines the exit-status verdict.
The response carries neither a verdict field nor a review ID: the process status
and the single requested review establish them.

## Version and explanation

The adapter echoes the request's `protocol_version`. Please accepts the supported
transition labels listed in the schema with identical validation, and rejects
unsupported versions and non-integer JSON version representations. There is no
fallback invocation. [api.yaml](api.yaml) records the emitted API version.

`reason` is a nonblank summary line, without control characters or Unicode line
and paragraph separators. `explanation` is optional or null; when supplied as text
it is nonblank and may span lines. The schema defines character and count bounds.
The numeric string bounds for explanation and finding messages also apply to
UTF-8 bytes during local validation; JSON Schema character counts alone cannot
establish these byte limits. The summary bound counts Unicode characters.

A conclusion that cannot be established as PASS, including insufficient essential
evidence or material uncertainty, is FAIL. ERROR denotes technical inability to
perform or validate the review. Please cannot enforce an adapter's reasoning,
and grounded citations do not prove that its conclusion is correct.

## Findings and local validation

The required `findings` array may be empty for either PASS or FAIL. Each finding
contains `message`, `path` and `line`; the last two may be null but must be present.
A cited path must identify a regular file in the verified project manifest. Its
line, when present, is positive and within that file's current text.

At `commit-msg`, a null path with a nonnull line cites the captured commit message.
It is valid only when that message is present and contains the line. At other
points, a line requires a file path. Both path and line may be null for a finding
without a source location.

Unknown response fields, wrong structural types, invalid evidence and exceeded
bounds make the call ERROR. Please verifies evidence and input stability locally;
a structurally valid response alone cannot establish PASS.

## Optional telemetry

`usage` is optional or null. The schema deliberately imposes no structural
restriction on this field: invalid telemetry never invalidates the verdict.
The recognized counters are `input_tokens`, `cached_input_tokens`, `output_tokens`,
`reasoning_output_tokens` and `cache_write_input_tokens`. Only positive integers
representable as unsigned 64-bit values count as reported. Other values, missing
keys and unknown counters are unavailable or ignored. Available usage is retained
even if the verdict's evidence is rejected.

Input plus output gives total tokens. Cached input, cache writes and reasoning
are subsets, not additional totals; consistency between counters is not validated.
A summed counter is available only when every adapter call reported it. Counters
are adapter claims, not billing, monetary cost or remaining quota.
