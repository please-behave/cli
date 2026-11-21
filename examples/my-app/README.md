# Illustrative My_App setup

From a macOS application's Git root, run `please init`, then copy the
`checks/build/` and `reviews/product/` directories into the corresponding
`.please/` directories. Set `project = "My_App"` in `.please/config.toml`.
Customize the Xcode project, scheme, destination and prompt before enabling.
Run `please sync` after editing and `please run` to execute enabled rules.
Both example Rules start disabled and remain validated. They run on the user's
machine with its permissions. The build Check requires macOS and Xcode.

### Optional attestation delivery

To send evidence for a current report with a commit, add this table to
`.please/config.toml`.

```toml
[publish]
gateway_url = "https://gateway.example.invalid"
key_env = "PLEASE_GATEWAY_KEY"
```

Set the key through your secret store, then run `please run` and
`please publish` as separate commands. The command uses the Git `origin` for
repository identity unless you provide an override.
[Publish settings](../../docs/configuration.md#publish-settings) describe the fields.
