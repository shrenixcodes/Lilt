# Lilt development workflow

Use `D:\Lilt Dev` on `dev/local-testing` for feature work. Keep `D:\Lilt` on `main` until the user explicitly asks to merge. Do not push or merge a test change as part of the testing cycle.

After every repository update, run `powershell -ExecutionPolicy Bypass -File .\tools\install-dev.ps1` from `D:\Lilt Dev`. The command runs formatting, Clippy, tests, a full release build with `dev-build`, and installs and starts Lilt Dev on this PC. Resolve failures and repeat the command before presenting the update for testing.

For each changed behavior, operate the installed Lilt Dev app on the user's PC and exercise the relevant flow. Check the visible result and, when useful, `%LOCALAPPDATA%\Lilt Dev\logs\lilt.log`. Include normal use and a relevant failure or edge case. Fix reproducible problems, rebuild, reinstall, and retest. If PC control or a required external condition is unavailable, report the exact verification gap rather than claiming the behavior works.

At handoff, state what changed, the build and test results, what was exercised on the PC, and any remaining limitation or confusion. Ask the user to test the installed build and tell you about any bugs they find; use that feedback to fix and repeat the cycle. Merge into `main` only after the user says the full test has passed and asks for the merge.
