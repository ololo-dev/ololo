use arena_core::entities::*;
use sea_orm::EntityName;

#[test]
fn judges_table_name() {
    assert_eq!(judges::Entity.table_name(), "judges");
}

#[test]
fn task_judges_table_name() {
    assert_eq!(task_judges::Entity.table_name(), "task_judges");
}
