# Create Commit

Create a git commit with the current changes.

## Steps

1. Run `git status` to see all changes
2. Run `git diff --staged` to see staged changes
3. If nothing is staged, run `git diff` to see unstaged changes
4. Analyze the changes to understand what was modified
5. Stage appropriate files with `git add`
6. Create a commit message that:
   - Uses conventional commit format (type(scope): description)
   - Is concise but descriptive
   - Explains the "why" not the "what"

7. Run `git commit` with the generated message

$ARGUMENTS
