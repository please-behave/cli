# Illustrative My_App setup

From a macOS application's Git root, run `please init`, then copy the
`checks/build/` and `reviews/product/` directories into the corresponding
`.please/` directories. Set `project = "My_App"` in `.please/config.toml`.
Customize the Xcode project, scheme, destination and prompt before enabling.
Run `please sync` after editing and `please run` to execute enabled rules.
Both example Rules start disabled and remain validated. They run on the user's
machine with its permissions. The build Check requires macOS and Xcode.
