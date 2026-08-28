//! Linking campaign parents to their parts.
//!
//! A campaign parent declares its parts by slug, in play order, in its seed
//! definition (`parts:` in readme frontmatter, `project.parts` in an export
//! envelope). This module turns that list into
//! `projects.parent_project_id_fk` + `projects.part_ordinal` on the children.
//!
//! Reconciliation is unlink-all-then-link inside one transaction: writing new
//! ordinals over live rows would trip `ux_projects_parent_part` halfway
//! through a reorder, and clearing first also gives "a part dropped from the
//! list becomes standalone again" for free.
//!
//! Two callers with deliberately different failure modes:
//! - boot seeding ([`link_parts_lenient`]) warns and skips a bad entry —
//!   startup must not die over a half-authored campaign;
//! - the admin API ([`link_parts_strict`]) rejects it, because someone is
//!   watching the response and a silently dropped part is a worse outcome
//!   than a 400.

use arena_core::entities::projects;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter,
    TransactionTrait,
};
use uuid::Uuid;

/// Why a declared part could not be linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkRejection {
    /// No project carries that slug.
    UnknownSlug(String),
    /// The parent listed itself.
    SelfReference(String),
    /// The slug appears twice in one part list.
    Duplicate(String),
    /// The part is itself a campaign parent — campaigns do not nest.
    NestedCampaign(String),
    /// Another campaign already claims this part.
    ClaimedElsewhere { slug: String, parent: String },
}

impl std::fmt::Display for LinkRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkRejection::UnknownSlug(s) => write!(f, "parts references unknown project '{s}'"),
            LinkRejection::SelfReference(s) => write!(f, "parts lists the campaign itself ('{s}')"),
            LinkRejection::Duplicate(s) => write!(f, "parts lists '{s}' twice"),
            LinkRejection::NestedCampaign(s) => {
                write!(f, "part '{s}' is itself a campaign; campaigns do not nest")
            }
            LinkRejection::ClaimedElsewhere { slug, parent } => {
                write!(f, "part '{slug}' already belongs to campaign '{parent}'")
            }
        }
    }
}

/// A part that passed validation, paired with the ordinal it will get.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPart {
    pub project_id: Uuid,
    pub slug: String,
    pub ordinal: i32,
}

/// Outcome of resolving a declared part list against the projects table.
#[derive(Debug, Clone, Default)]
pub struct Resolution {
    pub parts: Vec<ResolvedPart>,
    pub rejections: Vec<LinkRejection>,
}

/// Resolve `part_slugs` for `parent_id`, checking every rule. Ordinals are
/// assigned over the surviving entries (0..n) so a skipped slug leaves no gap.
pub async fn resolve_parts(
    db: &DatabaseConnection,
    parent_id: Uuid,
    parent_slug: &str,
    part_slugs: &[String],
) -> Result<Resolution, DbErr> {
    let mut out = Resolution::default();
    let mut seen: Vec<String> = Vec::new();

    for slug in part_slugs {
        if seen.iter().any(|s| s == slug) {
            out.rejections.push(LinkRejection::Duplicate(slug.clone()));
            continue;
        }
        seen.push(slug.clone());

        if slug == parent_slug {
            out.rejections
                .push(LinkRejection::SelfReference(slug.clone()));
            continue;
        }
        let Some(child) = projects::Entity::find()
            .filter(projects::Column::Slug.eq(slug))
            .one(db)
            .await?
        else {
            out.rejections
                .push(LinkRejection::UnknownSlug(slug.clone()));
            continue;
        };
        if child.id == parent_id {
            out.rejections
                .push(LinkRejection::SelfReference(slug.clone()));
            continue;
        }
        // Campaigns are depth-1: a part must not have parts of its own.
        let has_children = projects::Entity::find()
            .filter(projects::Column::ParentProjectIdFk.eq(child.id))
            .one(db)
            .await?
            .is_some();
        if has_children {
            out.rejections
                .push(LinkRejection::NestedCampaign(slug.clone()));
            continue;
        }
        // A part belongs to exactly one campaign. Re-linking to the same
        // parent is the normal reseed path and must stay allowed.
        if let Some(other_parent) = child.parent_project_id_fk
            && other_parent != parent_id
        {
            let parent_name = projects::Entity::find_by_id(other_parent)
                .one(db)
                .await?
                .and_then(|p| p.slug)
                .unwrap_or_else(|| other_parent.to_string());
            out.rejections.push(LinkRejection::ClaimedElsewhere {
                slug: slug.clone(),
                parent: parent_name,
            });
            continue;
        }

        let ordinal = out.parts.len() as i32;
        out.parts.push(ResolvedPart {
            project_id: child.id,
            slug: slug.clone(),
            ordinal,
        });
    }

    Ok(out)
}

/// Write the resolved links: clear the parent's current children, then stamp
/// the new list. Both steps share one transaction so a failure mid-way cannot
/// leave a campaign with half its parts detached.
pub async fn write_links(
    db: &DatabaseConnection,
    parent_id: Uuid,
    parts: &[ResolvedPart],
) -> Result<(), DbErr> {
    let txn = db.begin().await?;
    unlink_children(&txn, parent_id).await?;
    for part in parts {
        projects::Entity::update_many()
            .col_expr(
                projects::Column::ParentProjectIdFk,
                sea_orm::sea_query::Expr::value(Some(parent_id)),
            )
            .col_expr(
                projects::Column::PartOrdinal,
                sea_orm::sea_query::Expr::value(Some(part.ordinal)),
            )
            .filter(projects::Column::Id.eq(part.project_id))
            .exec(&txn)
            .await?;
    }
    txn.commit().await
}

/// Detach every part of `parent_id`. Also the stand-in for
/// `ON DELETE SET NULL` when a campaign parent is deleted (there is no
/// DB-level FK — see the migration).
pub async fn unlink_children<C: ConnectionTrait>(db: &C, parent_id: Uuid) -> Result<(), DbErr> {
    projects::Entity::update_many()
        .col_expr(
            projects::Column::ParentProjectIdFk,
            sea_orm::sea_query::Expr::value(None::<Uuid>),
        )
        .col_expr(
            projects::Column::PartOrdinal,
            sea_orm::sea_query::Expr::value(None::<i32>),
        )
        .filter(projects::Column::ParentProjectIdFk.eq(parent_id))
        .exec(db)
        .await?;
    Ok(())
}

/// Boot-seed linking: reconcile what resolves, warn about the rest.
pub async fn link_parts_lenient(
    db: &DatabaseConnection,
    parent_id: Uuid,
    parent_slug: &str,
    part_slugs: &[String],
) -> Result<usize, DbErr> {
    let resolution = resolve_parts(db, parent_id, parent_slug, part_slugs).await?;
    for rejection in &resolution.rejections {
        tracing::warn!(campaign = %parent_slug, "seed: {rejection}");
    }
    write_links(db, parent_id, &resolution.parts).await?;
    Ok(resolution.parts.len())
}

/// Admin-API linking: any rejection is an error, and nothing is written.
pub async fn link_parts_strict(
    db: &DatabaseConnection,
    parent_id: Uuid,
    parent_slug: &str,
    part_slugs: &[String],
) -> Result<usize, LinkError> {
    let resolution = resolve_parts(db, parent_id, parent_slug, part_slugs).await?;
    if let Some(first) = resolution.rejections.first() {
        return Err(LinkError::Rejected(first.clone()));
    }
    write_links(db, parent_id, &resolution.parts).await?;
    Ok(resolution.parts.len())
}

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("{0}")]
    Rejected(LinkRejection),
    #[error(transparent)]
    Db(#[from] DbErr),
}
