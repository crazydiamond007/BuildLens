import io
import zipfile

from buildlens_ai.redaction import extract_failure_ranges, redact, redact_data


def archive(files: dict[str, str]) -> bytes:
    target = io.BytesIO()
    with zipfile.ZipFile(target, "w") as output:
        for name, contents in files.items():
            output.writestr(name, contents)
    return target.getvalue()


def test_redacts_credentials_tokens_urls_and_emails() -> None:
    github_token = "ghp_" + "a" * 26
    value = redact(
        "Authorization: Bearer abc123 password=hunter2 "
        f"https://alice:secret@example.com owner@example.org {github_token}"
    )
    assert "abc123" not in value
    assert "hunter2" not in value
    assert "alice:secret" not in value
    assert "owner@example.org" not in value
    assert "ghp_" not in value
    assert value.count("[REDACTED") >= 4


def test_extracts_bounded_error_context_with_original_line_ranges() -> None:
    payload = archive(
        {
            "job/build.txt": "one\ntwo\nthree\nERROR token=supersecret\nfive\nsix\nseven",
            "job/ok.txt": "all good\nstill good",
        }
    )
    ranges = extract_failure_ranges(payload, max_lines=20, max_prompt_bytes=4096)
    assert len(ranges) == 1
    assert ranges[0].source == "job/build.txt"
    assert ranges[0].start_line == 2
    assert ranges[0].end_line == 6
    assert "supersecret" not in ranges[0].text


def test_skips_zip_traversal_paths() -> None:
    payload = archive({"../secret.txt": "fatal failure", "safe.txt": "error here"})
    ranges = extract_failure_ranges(payload, max_lines=20, max_prompt_bytes=4096)
    assert [item.source for item in ranges] == ["safe.txt"]


def test_recursively_redacts_structured_context() -> None:
    value = {
        "failure_message": "API_KEY=supersecret",
        "nested": ["owner@example.org", 42],
    }
    redacted = redact_data(value)
    assert "supersecret" not in redacted["failure_message"]
    assert redacted["nested"] == ["[REDACTED_EMAIL]", 42]
