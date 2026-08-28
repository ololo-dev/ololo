use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

fn escape(s: &str) -> String {
    s.replace('\'', "''")
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // ── users ──────────────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Users::Id).uuid().not_null().primary_key())
                    .col(
                        ColumnDef::new(Users::Email)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Users::PasswordHash).string().null())
                    .col(ColumnDef::new(Users::DisplayName).string().not_null())
                    .col(
                        ColumnDef::new(Users::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Users::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Users::IsAdmin)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(ColumnDef::new(Users::AvatarUrl).text().null())
                    .col(
                        ColumnDef::new(Users::EmailVerified)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(ColumnDef::new(Users::Username).text().null())
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_users_username")
                    .table(Users::Table)
                    .col(Users::Username)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // ── projects ───────────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Projects::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Projects::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Projects::Name).string().not_null())
                    .col(ColumnDef::new(Projects::Slug).text().null())
                    .col(
                        ColumnDef::new(Projects::Description)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    .col(ColumnDef::new(Projects::Category).text().null())
                    .col(
                        ColumnDef::new(Projects::Tags)
                            .text()
                            .not_null()
                            .default("[]"),
                    )
                    .col(ColumnDef::new(Projects::CoverImageUrl).text().null())
                    .col(ColumnDef::new(Projects::OwnerUserIdFk).uuid().not_null())
                    .col(
                        ColumnDef::new(Projects::Public)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Projects::ArchivedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Projects::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Projects::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_projects_owner")
                            .from(Projects::Table, Projects::OwnerUserIdFk)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        db.execute_unprepared(
            "CREATE UNIQUE INDEX idx_projects_owner_slug ON projects(owner_user_id_fk, slug)",
        )
        .await?;

        db.execute_unprepared(
            // `WHERE public` (not `= 1`): Postgres rejects boolean-to-integer
            // comparison; the bare boolean predicate works on both backends.
            "CREATE UNIQUE INDEX idx_projects_public_slug ON projects(slug) WHERE public",
        )
        .await?;

        // ── refresh_tokens ─────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(RefreshTokens::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RefreshTokens::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(RefreshTokens::UserIdFk).uuid().not_null())
                    .col(ColumnDef::new(RefreshTokens::Hash).string().not_null())
                    .col(
                        ColumnDef::new(RefreshTokens::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RefreshTokens::RevokedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(RefreshTokens::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_refresh_tokens_user")
                            .from(RefreshTokens::Table, RefreshTokens::UserIdFk)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // ── cli_tokens ─────────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(CliTokens::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(CliTokens::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(CliTokens::TokenHash).text().not_null())
                    .col(ColumnDef::new(CliTokens::UserId).uuid().not_null())
                    .col(
                        ColumnDef::new(CliTokens::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CliTokens::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_cli_tokens_user")
                            .from(CliTokens::Table, CliTokens::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_cli_tokens_token_hash")
                    .table(CliTokens::Table)
                    .col(CliTokens::TokenHash)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // ── auth_tokens ────────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(AuthTokens::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AuthTokens::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AuthTokens::UserId).uuid().not_null())
                    .col(ColumnDef::new(AuthTokens::TokenHash).text().not_null())
                    .col(
                        ColumnDef::new(AuthTokens::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AuthTokens::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AuthTokens::TokenType).text().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_auth_tokens_user")
                            .from(AuthTokens::Table, AuthTokens::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_auth_tokens_hash")
                    .table(AuthTokens::Table)
                    .col(AuthTokens::TokenHash)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // ── email_templates ────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(EmailTemplates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(EmailTemplates::Type)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(EmailTemplates::Subject).text().not_null())
                    .col(ColumnDef::new(EmailTemplates::BodyHtml).text().not_null())
                    .col(ColumnDef::new(EmailTemplates::BodyText).text().not_null())
                    .col(
                        ColumnDef::new(EmailTemplates::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        let verify_html = escape(include_str!("../../../email-templates/verify.html"));
        let verify_txt = escape(include_str!("../../../email-templates/verify.txt"));
        let reset_html = escape(include_str!("../../../email-templates/reset_password.html"));
        let reset_txt = escape(include_str!("../../../email-templates/reset_password.txt"));
        let magic_html = escape(include_str!("../../../email-templates/magic_link.html"));
        let magic_txt = escape(include_str!("../../../email-templates/magic_link.txt"));
        let now = "2026-05-24 00:00:00";

        db.execute_unprepared(&format!(
            "INSERT INTO email_templates (type, subject, body_html, body_text, updated_at) VALUES \
             ('verify', 'Verify your ololo.dev email address', '{verify_html}', '{verify_txt}', '{now}'), \
             ('reset_password', 'Reset your ololo.dev password', '{reset_html}', '{reset_txt}', '{now}'), \
             ('magic_link', 'Your ololo.dev sign-in link', '{magic_html}', '{magic_txt}', '{now}') \
             ON CONFLICT (type) DO NOTHING",
        ))
        .await?;

        // ── app_settings ───────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(AppSettings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AppSettings::Key)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AppSettings::Value).string().not_null())
                    .to_owned(),
            )
            .await?;

        db.execute_unprepared(
            "INSERT INTO app_settings (key, value) VALUES \
             ('ai_provider', 'ollama'), \
             ('ai_model', 'llama3.2'), \
             ('allow_user_project_creation', 'true') \
             ON CONFLICT (key) DO NOTHING",
        )
        .await?;

        // ── categories ─────────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Categories::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Categories::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Categories::Name)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(Categories::Ordinal)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;

        db.execute_unprepared(
            "INSERT INTO categories (name, ordinal) VALUES \
             ('Algorithms', 0), ('Data Structures', 1), ('Math', 2), \
             ('Web', 3), ('API', 4), ('CLI', 5), ('Game', 6), \
             ('Data Science', 7), ('Other', 8) \
             ON CONFLICT (name) DO NOTHING",
        )
        .await?;

        // ── game_servers ───────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(GameServers::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(GameServers::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(GameServers::Url).text().not_null())
                    .col(ColumnDef::new(GameServers::DisplayName).text().null())
                    .col(
                        ColumnDef::new(GameServers::Capacity)
                            .integer()
                            .not_null()
                            .default(10),
                    )
                    .col(
                        ColumnDef::new(GameServers::ActiveSessions)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(GameServers::Status)
                            .text()
                            .not_null()
                            .default("active"),
                    )
                    .col(
                        ColumnDef::new(GameServers::LastHeartbeat)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GameServers::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(GameServers::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ── sessions ───────────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Sessions::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Sessions::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Sessions::Name).string().not_null())
                    .col(
                        ColumnDef::new(Sessions::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Sessions::OwnerIdFk).uuid().null())
                    .col(
                        ColumnDef::new(Sessions::Status)
                            .string()
                            .not_null()
                            .default("lobby"),
                    )
                    .col(ColumnDef::new(Sessions::JoinCode).string().not_null())
                    .col(
                        ColumnDef::new(Sessions::StartedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Sessions::FinishedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(ColumnDef::new(Sessions::ProjectIdFk).uuid().not_null())
                    .col(ColumnDef::new(Sessions::GameServerId).uuid().null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_sessions_project")
                            .from(Sessions::Table, Sessions::ProjectIdFk)
                            .to(Projects::Table, Projects::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_sessions_game_server")
                            .from(Sessions::Table, Sessions::GameServerId)
                            .to(GameServers::Table, GameServers::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_sessions_join_code")
                    .table(Sessions::Table)
                    .col(Sessions::JoinCode)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_project_status")
                    .table(Sessions::Table)
                    .col(Sessions::ProjectIdFk)
                    .col(Sessions::Status)
                    .to_owned(),
            )
            .await?;

        // ── tasks ──────────────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Tasks::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Tasks::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Tasks::ProjectIdFk).uuid().not_null())
                    .col(ColumnDef::new(Tasks::Ordinal).integer().not_null())
                    .col(ColumnDef::new(Tasks::Title).string().not_null())
                    .col(ColumnDef::new(Tasks::Content).text().not_null())
                    .col(ColumnDef::new(Tasks::TestTemplate).json_binary().not_null())
                    .col(
                        ColumnDef::new(Tasks::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Tasks::Tags).text().not_null().default("[]"))
                    .col(
                        ColumnDef::new(Tasks::PointValue)
                            .integer()
                            .not_null()
                            .default(10),
                    )
                    .col(
                        ColumnDef::new(Tasks::DeadlineSecs)
                            .big_integer()
                            .not_null()
                            .default(300),
                    )
                    .col(
                        ColumnDef::new(Tasks::MinIntervalSecs)
                            .integer()
                            .not_null()
                            .default(5),
                    )
                    .col(
                        ColumnDef::new(Tasks::IntervalIncrementSecs)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Tasks::MaxIntervalSecs)
                            .integer()
                            .not_null()
                            .default(300),
                    )
                    .col(
                        ColumnDef::new(Tasks::PassPoints)
                            .integer()
                            .not_null()
                            .default(1),
                    )
                    .col(
                        ColumnDef::new(Tasks::FailPoints)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Tasks::NoResponsePoints)
                            .integer()
                            .not_null()
                            .default(-1),
                    )
                    .col(
                        ColumnDef::new(Tasks::CompletionBonusPoints)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_tasks_project")
                            .from(Tasks::Table, Tasks::ProjectIdFk)
                            .to(Projects::Table, Projects::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ux_tasks_project_ordinal")
                    .table(Tasks::Table)
                    .col(Tasks::ProjectIdFk)
                    .col(Tasks::Ordinal)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // ── players ────────────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Players::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Players::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Players::SessionIdFk).uuid().not_null())
                    .col(ColumnDef::new(Players::UserIdFk).uuid().null())
                    .col(ColumnDef::new(Players::DisplayName).string().not_null())
                    .col(ColumnDef::new(Players::Fingerprint).string().null())
                    .col(ColumnDef::new(Players::MetadataJson).text().null())
                    .col(
                        ColumnDef::new(Players::JoinedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Players::ReconnectedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Players::RevokedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_players_session")
                            .from(Players::Table, Players::SessionIdFk)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_players_user")
                            .from(Players::Table, Players::UserIdFk)
                            .to(Users::Table, Users::Id),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_players_session")
                    .table(Players::Table)
                    .col(Players::SessionIdFk)
                    .to_owned(),
            )
            .await?;

        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_players_fingerprint \
             ON players (session_id_fk, fingerprint) \
             WHERE fingerprint IS NOT NULL",
        )
        .await?;

        // ── tests ──────────────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Tests::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Tests::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Tests::CommandTemplate).text().not_null())
                    .col(ColumnDef::new(Tests::AnswerTemplate).text().not_null())
                    .col(ColumnDef::new(Tests::FixtureDefinitions).text().not_null())
                    .col(
                        ColumnDef::new(Tests::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Tests::SessionId).uuid().not_null())
                    .col(ColumnDef::new(Tests::TaskId).uuid().not_null())
                    .col(
                        ColumnDef::new(Tests::Ordinal)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(Tests::Prompt).text().not_null().default(""))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_tests_session")
                            .from(Tests::Table, Tests::SessionId)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_tests_task")
                            .from(Tests::Table, Tests::TaskId)
                            .to(Tasks::Table, Tasks::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ux_tests_session_task_ordinal")
                    .table(Tests::Table)
                    .col(Tests::SessionId)
                    .col(Tests::TaskId)
                    .col(Tests::Ordinal)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // ── task_results ───────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(TaskResults::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(TaskResults::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(TaskResults::SessionIdFk).uuid().not_null())
                    .col(ColumnDef::new(TaskResults::PlayerIdFk).uuid().not_null())
                    .col(ColumnDef::new(TaskResults::TaskId).uuid().null())
                    .col(ColumnDef::new(TaskResults::Answer).text().not_null())
                    .col(
                        ColumnDef::new(TaskResults::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TaskResults::PointDelta)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(TaskResults::IsBonus)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_task_results_session")
                            .from(TaskResults::Table, TaskResults::SessionIdFk)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_task_results_player")
                            .from(TaskResults::Table, TaskResults::PlayerIdFk)
                            .to(Players::Table, Players::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_task_results_task")
                            .from(TaskResults::Table, TaskResults::TaskId)
                            .to(Tasks::Table, Tasks::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_task_results_session_player")
                    .table(TaskResults::Table)
                    .col(TaskResults::SessionIdFk)
                    .col(TaskResults::PlayerIdFk)
                    .to_owned(),
            )
            .await?;

        // ── session_scheduler_state ────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(SessionSchedulerState::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SessionSchedulerState::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(SessionSchedulerState::SessionIdFk)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SessionSchedulerState::PlayerIdFk)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(SessionSchedulerState::TaskId).uuid().null())
                    .col(
                        ColumnDef::new(SessionSchedulerState::State)
                            .text()
                            .not_null()
                            .default("idle"),
                    )
                    .col(
                        ColumnDef::new(SessionSchedulerState::NextProbeAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(SessionSchedulerState::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SessionSchedulerState::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_sched_state_session")
                            .from(
                                SessionSchedulerState::Table,
                                SessionSchedulerState::SessionIdFk,
                            )
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_sched_state_player")
                            .from(
                                SessionSchedulerState::Table,
                                SessionSchedulerState::PlayerIdFk,
                            )
                            .to(Players::Table, Players::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_sched_state_task")
                            .from(SessionSchedulerState::Table, SessionSchedulerState::TaskId)
                            .to(Tasks::Table, Tasks::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        // ── probes ─────────────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Probes::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Probes::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Probes::TestId).uuid().not_null())
                    .col(ColumnDef::new(Probes::PlayerId).uuid().not_null())
                    .col(ColumnDef::new(Probes::SessionId).uuid().not_null())
                    .col(
                        ColumnDef::new(Probes::Attempt)
                            .integer()
                            .not_null()
                            .default(1),
                    )
                    .col(ColumnDef::new(Probes::RenderedCommand).text().not_null())
                    .col(ColumnDef::new(Probes::FixtureValues).text().not_null())
                    .col(ColumnDef::new(Probes::ExpectedAnswer).text().null())
                    .col(ColumnDef::new(Probes::Outcome).string().null())
                    .col(
                        ColumnDef::new(Probes::DispatchedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Probes::DeadlineAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Probes::ResolvedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Probes::UpdatedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(ColumnDef::new(Probes::Output).text().null())
                    .col(ColumnDef::new(Probes::ExitCode).integer().null())
                    .col(ColumnDef::new(Probes::DurationMs).big_integer().null())
                    .col(ColumnDef::new(Probes::PointDelta).integer().null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_probes_test")
                            .from(Probes::Table, Probes::TestId)
                            .to(Tests::Table, Tests::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_probes_player")
                            .from(Probes::Table, Probes::PlayerId)
                            .to(Players::Table, Players::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_probes_session")
                            .from(Probes::Table, Probes::SessionId)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_probes_player_outcome")
                    .table(Probes::Table)
                    .col(Probes::PlayerId)
                    .col(Probes::Outcome)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_probes_session")
                    .table(Probes::Table)
                    .col(Probes::SessionId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Probes::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(SessionSchedulerState::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(TaskResults::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Tests::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Players::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Tasks::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Sessions::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(GameServers::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(Categories::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(AppSettings::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(EmailTemplates::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(AuthTokens::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(CliTokens::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(RefreshTokens::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Projects::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Users::Table).if_exists().to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    PasswordHash,
    DisplayName,
    CreatedAt,
    UpdatedAt,
    IsAdmin,
    AvatarUrl,
    EmailVerified,
    Username,
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    Id,
    Name,
    Slug,
    Description,
    Category,
    Tags,
    CoverImageUrl,
    OwnerUserIdFk,
    Public,
    ArchivedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum RefreshTokens {
    Table,
    Id,
    UserIdFk,
    Hash,
    ExpiresAt,
    RevokedAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum CliTokens {
    Table,
    Id,
    TokenHash,
    UserId,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum AuthTokens {
    Table,
    Id,
    UserId,
    TokenHash,
    ExpiresAt,
    CreatedAt,
    TokenType,
}

#[derive(DeriveIden)]
enum EmailTemplates {
    Table,
    Type,
    Subject,
    BodyHtml,
    BodyText,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum AppSettings {
    Table,
    Key,
    Value,
}

#[derive(DeriveIden)]
enum Categories {
    Table,
    Id,
    Name,
    Ordinal,
}

#[derive(DeriveIden)]
enum GameServers {
    Table,
    Id,
    Url,
    DisplayName,
    Capacity,
    ActiveSessions,
    Status,
    LastHeartbeat,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
    Name,
    CreatedAt,
    OwnerIdFk,
    Status,
    JoinCode,
    StartedAt,
    FinishedAt,
    ProjectIdFk,
    GameServerId,
}

#[derive(DeriveIden)]
enum Tasks {
    Table,
    Id,
    ProjectIdFk,
    Ordinal,
    Title,
    Content,
    TestTemplate,
    CreatedAt,
    Tags,
    PointValue,
    DeadlineSecs,

    MinIntervalSecs,
    IntervalIncrementSecs,
    MaxIntervalSecs,
    PassPoints,
    FailPoints,
    NoResponsePoints,
    CompletionBonusPoints,
}

#[derive(DeriveIden)]
enum Players {
    Table,
    Id,
    SessionIdFk,
    UserIdFk,
    DisplayName,
    Fingerprint,
    MetadataJson,
    JoinedAt,
    ReconnectedAt,
    RevokedAt,
}

#[derive(DeriveIden)]
enum Tests {
    Table,
    Id,
    CommandTemplate,
    AnswerTemplate,
    FixtureDefinitions,
    CreatedAt,
    SessionId,
    TaskId,
    Ordinal,
    Prompt,
}

#[derive(DeriveIden)]
enum TaskResults {
    Table,
    Id,
    SessionIdFk,
    PlayerIdFk,
    TaskId,
    Answer,
    CreatedAt,
    PointDelta,
    IsBonus,
}

#[derive(DeriveIden)]
enum SessionSchedulerState {
    Table,
    Id,
    SessionIdFk,
    PlayerIdFk,
    TaskId,
    State,
    NextProbeAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Probes {
    Table,
    Id,
    TestId,
    PlayerId,
    SessionId,
    Attempt,
    RenderedCommand,
    FixtureValues,
    ExpectedAnswer,
    Outcome,
    DispatchedAt,
    DeadlineAt,
    ResolvedAt,
    UpdatedAt,
    Output,
    ExitCode,
    DurationMs,
    PointDelta,
}
