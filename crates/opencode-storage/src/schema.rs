// ============================================================================
// SQLite Schema Definitions
// Based on TypeScript: /opencode/packages/opencode/src/session/session.sql.ts
// ============================================================================

/// Sessions table - stores session metadata
pub const CREATE_SESSIONS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    parent_id TEXT,
    slug TEXT NOT NULL,
    directory TEXT NOT NULL,
    title TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '1.0.0',
    share_url TEXT,
    
    -- Summary fields
    summary_additions INTEGER DEFAULT 0,
    summary_deletions INTEGER DEFAULT 0,
    summary_files INTEGER DEFAULT 0,
    summary_diffs TEXT,
    
    -- Revert info (JSON)
    revert TEXT,
    
    -- Permission ruleset (JSON)
    permission TEXT,
    
    -- Usage stats
    usage_input_tokens INTEGER DEFAULT 0,
    usage_output_tokens INTEGER DEFAULT 0,
    usage_reasoning_tokens INTEGER DEFAULT 0,
    usage_cache_write_tokens INTEGER DEFAULT 0,
    usage_cache_read_tokens INTEGER DEFAULT 0,
    usage_total_cost REAL DEFAULT 0.0,
    
    -- Status
    status TEXT NOT NULL DEFAULT 'active',
    
    -- Timestamps (milliseconds since epoch)
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    time_compacting INTEGER,
    time_archived INTEGER
);
"#;

/// Messages table - stores message metadata
pub const CREATE_MESSAGES_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    
    -- Provider/model info
    provider_id TEXT,
    model_id TEXT,
    
    -- Token usage
    tokens_input INTEGER DEFAULT 0,
    tokens_output INTEGER DEFAULT 0,
    tokens_reasoning INTEGER DEFAULT 0,
    tokens_cache_read INTEGER DEFAULT 0,
    tokens_cache_write INTEGER DEFAULT 0,
    cost REAL DEFAULT 0.0,
    
    -- Complete message data (JSON)
    data TEXT,
    
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
"#;

/// Parts table - stores message parts (text, tool calls, etc.)
pub const CREATE_PARTS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS parts (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    
    -- Part type
    part_type TEXT NOT NULL,
    
    -- Text content
    text TEXT,
    
    -- Tool call fields
    tool_name TEXT,
    tool_call_id TEXT,
    tool_arguments TEXT,
    tool_result TEXT,
    tool_error TEXT,
    tool_status TEXT,
    
    -- File fields
    file_url TEXT,
    file_filename TEXT,
    file_mime TEXT,
    
    -- Reasoning fields
    reasoning TEXT,
    
    -- Sort order
    sort_order INTEGER NOT NULL DEFAULT 0,
    
    -- Complete part data (JSON)
    data TEXT,
    
    FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
"#;

/// Todos table - stores session todos
pub const CREATE_TODOS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS todos (
    session_id TEXT NOT NULL,
    todo_id TEXT NOT NULL,
    content TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    priority TEXT NOT NULL DEFAULT 'medium',
    position INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    
    PRIMARY KEY (session_id, todo_id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
"#;

/// Permissions table - stores project-level permissions
pub const CREATE_PERMISSIONS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS permissions (
    project_id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    data TEXT NOT NULL
);
"#;

/// Session shares table - stores share info for sessions
pub const CREATE_SESSION_SHARES_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS session_shares (
    session_id TEXT PRIMARY KEY,
    id TEXT NOT NULL,
    secret TEXT NOT NULL,
    url TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
"#;

/// Create indexes for better query performance
pub const CREATE_INDEXES: &str = r#"
-- Session indexes
CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id);
CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_id);
CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);

-- Message indexes
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at);

-- Part indexes
CREATE INDEX IF NOT EXISTS idx_parts_message ON parts(message_id);
CREATE INDEX IF NOT EXISTS idx_parts_session ON parts(session_id);
CREATE INDEX IF NOT EXISTS idx_parts_order ON parts(sort_order);

-- Todo indexes
CREATE INDEX IF NOT EXISTS idx_todos_session ON todos(session_id);
CREATE INDEX IF NOT EXISTS idx_todos_status ON todos(status);
"#;

/// All migration statements to run
pub const ALL_MIGRATIONS: &[&str] = &[
    CREATE_SESSIONS_TABLE,
    CREATE_MESSAGES_TABLE,
    CREATE_PARTS_TABLE,
    CREATE_TODOS_TABLE,
    CREATE_PERMISSIONS_TABLE,
    CREATE_SESSION_SHARES_TABLE,
    CREATE_INDEXES,
];
