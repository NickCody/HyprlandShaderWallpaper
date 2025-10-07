# GitHub Branch Protection Configuration

This document explains how to configure GitHub branch protection rules to ensure Pull Requests cannot be merged unless CI checks pass.

## Quick Setup

Repository administrators should configure the following branch protection rules for the `main` branch:

### Step 1: Navigate to Branch Protection Settings

1. Go to the repository on GitHub: https://github.com/NickCody/WallShader
2. Click **Settings** (requires admin access)
3. In the left sidebar, click **Branches**
4. Under "Branch protection rules", click **Add rule** or edit the existing rule for `main`

### Step 2: Configure Protection Rules

Set the following options for the `main` branch:

#### Required Settings

- **Branch name pattern**: `main`
- **✅ Require a pull request before merging**
  - Optional: Check "Require approvals" (set to 1 or more reviewers)
  - Optional: Check "Dismiss stale pull request approvals when new commits are pushed"
- **✅ Require status checks to pass before merging**
  - **✅ Require branches to be up to date before merging**
  - Under "Status checks that are required", add:
    - `Test and Quality Checks` (from the CI workflow)
    - `Build AppImage` (from the CI workflow)
- **✅ Require conversation resolution before merging** (recommended)
- Optional: **✅ Require signed commits**
- Optional: **✅ Include administrators** (apply rules to admins too)

#### Optional but Recommended Settings

- **✅ Require linear history** - Prevents merge commits, enforces rebase or squash
- **✅ Do not allow bypassing the above settings**

### Step 3: Save Changes

Click **Create** or **Save changes** at the bottom of the page.

## What This Does

Once configured, the following rules will be enforced:

1. **No direct pushes to main**: All changes must go through a Pull Request
2. **CI must pass**: The "Test and Quality Checks" job must succeed, which includes:
   - Code formatting check (`cargo fmt --all --check`)
   - Linting with Clippy (`cargo clippy --all-targets --all-features -- -D warnings`)
   - Full build (`cargo build --verbose`)
   - All tests (`cargo test --verbose`)
3. **AppImage must build**: The AppImage build process must complete successfully
4. **Branch must be up-to-date**: PR branches must be updated with the latest main before merging
5. **Conversations must be resolved**: All review comments must be addressed

## CI Workflow Jobs

The repository has a CI workflow (`.github/workflows/ci.yml`) with two jobs:

### 1. Test and Quality Checks
This job runs on every PR and includes:
- Formatting validation
- Clippy linting (with warnings treated as errors)
- Full build
- Complete test suite

### 2. Build AppImage
This job:
- Depends on "Test and Quality Checks" passing
- Builds the release binary
- Creates the AppImage package
- Runs basic smoke tests
- Uploads artifacts

## Verification

After configuring branch protection, test it by:

1. Creating a test branch with a small change
2. Opening a PR to `main`
3. Attempting to merge before CI completes (should be blocked)
4. Waiting for CI to pass
5. Verifying the merge button becomes available

## Troubleshooting

### Status checks not appearing in the list

If the status check names don't appear in the dropdown:
1. The workflow needs to run at least once on a PR
2. Create a test PR to trigger the workflow
3. Once it runs, the check names will appear in the branch protection settings

### CI workflow not running

Ensure:
- The workflow file is on the `main` branch
- The workflow has `pull_request:` triggers configured
- Actions are enabled for the repository (Settings → Actions → General)

### Administrators can still push

To enforce rules for administrators too:
- Enable "Include administrators" in the branch protection settings
- This applies all rules to admin users as well

## Additional Resources

- [CONTRIBUTING.md](../CONTRIBUTING.md) - Complete guide for contributors, including PR process and coding standards
- [Pull Request Template](pull_request_template.md) - Template used for all new PRs
- [GitHub Branch Protection Documentation](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)
- [Required Status Checks](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches#require-status-checks-before-merging)
- [GitHub Actions CI/CD](https://docs.github.com/en/actions/automating-builds-and-tests/about-continuous-integration)
