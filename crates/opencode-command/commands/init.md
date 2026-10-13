# Initialize OpenCode

Initialize OpenCode in the current project. This will:

1. Analyze the project structure
2. Create `.opencode/` directory if needed
3. Set up project-specific configuration

## Steps

1. First, explore the project structure to understand what kind of project this is:
   - Check for `package.json` (Node.js/JavaScript)
   - Check for `Cargo.toml` (Rust)
   - Check for `pyproject.toml` or `requirements.txt` (Python)
   - Check for `go.mod` (Go)
   - Check for `pom.xml` or `build.gradle` (Java)

2. Look at the existing code structure and identify:
   - Main source directories
   - Test directories
   - Configuration files
   - Build scripts

3. Create or update `.opencode/settings.json` with project-specific settings

4. Provide a summary of the project structure and any recommendations

$ARGUMENTS
