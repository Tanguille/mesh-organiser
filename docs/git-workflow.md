# Git Workflow Best Practices

## Use a Fresh Worktree for Each Task

To avoid accidentally committing unrelated changes, create a new git worktree for each task:

```bash
# Create a new worktree from main for your task
git worktree add ../mesh-organiser-task -b feature/your-task-name

# Work on your task in the new directory
cd ../mesh-organiser-task

# When done, delete the worktree and branch
cd ..
git worktree remove mesh-organiser-task
git branch -D feature/your-task-name
```

This ensures:

- Each task starts from a clean main branch state
- No uncommitted or unrelated changes leak into your commits
- Multiple tasks can be worked on in parallel without interference

## Dependency Files

**Do NOT** commit changes to these files unless the task explicitly involves updating dependencies:

- package.json
- package-lock.json
- Cargo.lock
- mise.toml
