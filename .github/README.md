# GitHub Configuration and Documentation

This directory contains GitHub-specific configuration files and documentation for the wax11 shader project.

## Files in this Directory

### Workflows
- **`workflows/ci.yml`** - Main CI/CD workflow that runs on every PR and push
  - Runs formatting checks, linting, builds, and tests
  - Builds AppImage releases for tagged versions
  - All checks must pass before PRs can be merged

- **`workflows/cache-optimization.yml`** - Automated cache cleanup
  - Runs weekly to prevent unlimited cache growth

### Documentation

- **`BRANCH_PROTECTION.md`** - Step-by-step guide for repository administrators
  - How to configure GitHub branch protection rules
  - Ensures PRs cannot be merged unless CI passes
  - Includes verification steps and troubleshooting

- **`pull_request_template.md`** - Template for new pull requests
  - Used automatically when creating a PR
  - Ensures consistent PR descriptions
  - Includes checklist for contributors

## For Contributors

If you're contributing to wax11 shader:

1. **Read the [CONTRIBUTING.md](../CONTRIBUTING.md)** in the repository root
2. Use the **pull request template** when opening PRs (it's applied automatically)
3. Ensure all CI checks pass (see CI status in your PR)
4. Address code review feedback

## For Repository Administrators

If you need to configure branch protection:

1. **Read [BRANCH_PROTECTION.md](BRANCH_PROTECTION.md)** for detailed setup instructions
2. Configure the rules in GitHub Settings → Branches
3. Verify the configuration by testing a PR

## CI Status

The CI workflow runs automatically on:
- Every push to `main`
- Every pull request to `main`
- Manual triggers (via GitHub Actions UI)
- Release tags (builds and publishes AppImage)

View the current CI status: [GitHub Actions](https://github.com/NickCody/wax11 shader/actions)
