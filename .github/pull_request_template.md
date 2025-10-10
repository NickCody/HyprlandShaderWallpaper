## Description

<!-- Provide a clear and concise description of your changes -->

## Motivation and Context

<!-- Why is this change needed? What problem does it solve? -->
<!-- If it fixes an issue, link it here: Fixes #123 -->

## Type of Change

<!-- Mark the relevant option with an "x" -->

- [ ] Bug fix (non-breaking change that fixes an issue)
- [ ] New feature (non-breaking change that adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] Documentation update
- [ ] Performance improvement
- [ ] Code refactoring
- [ ] Build/CI configuration

## Testing Performed

<!-- Describe the tests you ran to verify your changes -->

- [ ] Ran `just validate` locally (build + tests + clippy)
- [ ] Tested with `just run-demo`
- [ ] Tested with `just run-playlist`
- [ ] Added new tests for the changes
- [ ] All existing tests pass

### Manual Testing

<!-- Describe any manual testing you performed -->

```bash
# Example commands you used for testing
wax11 --window --shadertoy https://www.shadertoy.com/view/3dXyWj
```

## Screenshots/Demos

<!-- If your changes affect the UI or visual output, include screenshots or video links -->

## Checklist

<!-- Go through this checklist before submitting your PR -->

- [ ] My code follows the project's code style (`cargo fmt --all`)
- [ ] I have run `cargo clippy` and resolved all warnings
- [ ] I have added tests that prove my fix is effective or that my feature works
- [ ] All new and existing tests pass (`cargo test --verbose`)
- [ ] I have updated the documentation (if applicable)
- [ ] I have added an entry to CHANGELOG.md (for notable changes)
- [ ] My commits follow the project's commit message conventions
- [ ] I have rebased my branch on the latest main

## Additional Notes

<!-- Any additional information that reviewers should know -->
