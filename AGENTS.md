# AI Agent Guidelines

## GitHub

### Project

All issues for this project should be included in the [spotuify project](https://github.com/users/joegoggin/projects/25)
This project consists of three different fields:

#### Labels

- Feature
- Bug
- DevOps
- Documentation
- Refactor
- Testing
- Update

#### Status

- Todo
- In Progress
- Done

#### Priority

- Low 
- Medium
- High
- Urgent

### Issues

#### Creating Issues

When creating issues on GitHub for this project you should use the following
conventions:

- All new issues should be given a status of `Todo`
- If a priority isn't provided set the priority to `Medium` by default 
- Create a main issue with a summary of the full task that needs to be completed
- Break up the full task into small tasks and add those as sub-issues
- Each sub task should have a summary of the small task and match the priority
  of the main issue
- The order of the sub-task should be in the order the tasks should be
  implemented

#### Implementing Issues

When asked to implement an issue you should do the following:

- Read the issue for context
    - If issue is a sub-issue read the main issue for context
- Update the status of the issue to `In Progress`
- Implement the task    
- Describe what you did and provide steps to test  

#### Providing Instruction For Issues

When asked to provide instructions for implementing an issue you should do the
following:

- Read the issue for context
    - If issue is a sub-issue read the main issue for context 
- Come up with a plan to implement the issues
    - This plan should always default to using `just` commands if they exist
- Write the plan to a file called `issue-*.md` where `*` is the issue number
    - If asked to create instructions for multiple issues or for the sub-issues
      of a main issue ensure the each individual issue has there own file
    - DO NOT include multiple issues in one file
    - If file matching the pattern for an issue already exists DO NOT recreate a
      plan for that issue
    - These files should be stored in the `issues` directory
- Include detailed code examples for each step
- Include steps for manually testing the changes
- DO NOT implement the plan 
- DO NOT create instructions unless specifically asked

#### Updating Issues

When asked to reevaluate the plan or update issues you should do the following:

- Review recent changes for context
- Compare them to existing `issue-*.md` files and address any inconsistencies
  cause by the changes if needed
    - What to look for:
        - Project structure changes
        - Code style/convention changes
        - Variable name changes
- Compare the updated `issue-*.md` to the existing GitHub issue to ensure they still
  match each other
- DO NOT implement the plan


## Git

When working with git you should follow these conventions:

- NEVER commit or push to `main`
- If asked to push to `main` prompt me about creating a branch
- NEVER create a new branch without my permission

## Merge Conflict Resolution Process

When asked to help resolve merge conflicts, follow this interactive process:

1. Identify all conflicted files first.
2. Work through conflicts one at a time (do not resolve all at once in a single response).
3. For each conflict:
   - Explain what each side of the conflict is doing.
   - Propose a specific fix with a diff-style snippet.
   - Ask the user to accept or reject the proposed change.
4. Wait for user confirmation before applying each conflict resolution.
5. If accepted, apply the change; if rejected, skip and propose the next conflict.
6. After all conflicts are addressed:
   - Verify no merge markers remain (`<<<<<<<`, `=======`, `>>>>>>>`) in project files.
   - Verify no files remain in unmerged (`UU`) state.
   - Stage resolved files.
   - Run relevant checks/build commands when possible and report results.
7. After conflict resolution is complete, ask whether to:
   - Commit the merge resolution
   - Push the branch
   - Create or update a PR summary

