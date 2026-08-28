//! Named model pools: an assignment can point at a pool instead of a single
//! provider+model.
//!
//! A pool is an ordered set of candidates grouped into priority tiers. Members
//! sharing a `priority` form one tier and split the load between them, which is
//! where the throughput comes from; a tier is only left behind when its members
//! fail, and resolution then drops to the next-higher `priority` value. Storing
//! pools as rows (rather than inlining a list in each assignment) lets one pool
//! back several operations and judges at once.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(LlmPools::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(LlmPools::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(LlmPools::Name).text().not_null())
                    .col(
                        ColumnDef::new(LlmPools::Description)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    // Round-robin cursor, bumped once per resolve. Load is
                    // split by rotating each tier's start position by this
                    // value, so the two server processes share one sequence
                    // instead of each keeping a private in-memory counter.
                    .col(
                        ColumnDef::new(LlmPools::RrCursor)
                            .big_integer()
                            .not_null()
                            .default(0i64),
                    )
                    .col(
                        ColumnDef::new(LlmPools::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LlmPools::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Names identify a pool in the admin UI; keeping them unique stops two
        // pools from being indistinguishable there.
        manager
            .create_index(
                Index::create()
                    .name("ux_llm_pools_name")
                    .table(LlmPools::Table)
                    .col(LlmPools::Name)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(LlmPoolMembers::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(LlmPoolMembers::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(LlmPoolMembers::PoolIdFk).uuid().not_null())
                    .col(
                        ColumnDef::new(LlmPoolMembers::ProviderIdFk)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(LlmPoolMembers::Model).text().not_null())
                    // Lower value = tried first. Members sharing a value form
                    // one tier and share the load.
                    .col(
                        ColumnDef::new(LlmPoolMembers::Priority)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(LlmPoolMembers::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(LlmPoolMembers::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LlmPoolMembers::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_llm_pool_members_pool")
                            .from(LlmPoolMembers::Table, LlmPoolMembers::PoolIdFk)
                            .to(LlmPools::Table, LlmPools::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    // Deleting a provider removes it from every pool rather
                    // than leaving a member that can never resolve.
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_llm_pool_members_provider")
                            .from(LlmPoolMembers::Table, LlmPoolMembers::ProviderIdFk)
                            .to(LlmProviders::Table, LlmProviders::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_llm_pool_members_pool")
                    .table(LlmPoolMembers::Table)
                    .col(LlmPoolMembers::PoolIdFk)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(LlmPoolMembers::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(LlmPools::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum LlmPools {
    Table,
    Id,
    Name,
    Description,
    RrCursor,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum LlmPoolMembers {
    Table,
    Id,
    PoolIdFk,
    ProviderIdFk,
    Model,
    Priority,
    Enabled,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum LlmProviders {
    Table,
    Id,
}
