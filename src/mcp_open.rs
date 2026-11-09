use crate::ipc::OpenedCheck;
use crate::session_folder::SessionName;
use crate::test_files::TestFileList;
use rmcp::model::CallToolResult;
use serde_json::json;

pub(super) fn opened_result(
    session: &SessionName,
    checks: &[OpenedCheck],
    warnings: &[String],
    credentials: &[String],
    files: &TestFileList,
) -> CallToolResult {
    CallToolResult::structured(
        json!({"session":session,"checks":checks,"warnings":warnings,"credentials":credentials,"files":files.names,"more_files":files.more}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_session_reply_carries_files_and_the_count_of_the_rest_next_to_credentials() {
        let session = "app_12-00-00".parse().unwrap();
        let checks = [OpenedCheck {
            id: crate::timeline::CheckId(1),
            text: "upload works".into(),
        }];
        let result = opened_result(
            &session,
            &checks,
            &["warning".into()],
            &["PASSWORD".into()],
            &TestFileList {
                names: vec!["docs/upload.pdf".into()],
                more: 3,
            },
        );
        assert_eq!(
            result.structured_content.unwrap(),
            json!({"session":"app_12-00-00","checks":[{"id":1,"text":"upload works"}],"warnings":["warning"],"credentials":["PASSWORD"],"files":["docs/upload.pdf"],"more_files":3})
        );
    }

    #[test]
    fn a_project_without_test_files_replies_with_an_empty_files_list_and_more_files_0() {
        let session = "app_12-00-00".parse().unwrap();
        let result = opened_result(&session, &[], &[], &[], &TestFileList::default());
        let value = result.structured_content.unwrap();
        assert_eq!(value["files"], json!([]));
        assert_eq!(value["more_files"], 0);
    }
}
