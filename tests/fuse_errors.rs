use fuser::Errno;
use geesefs_rs::{error::Error, fuse::errno};

#[test]
fn filesystem_and_dependency_failures_preserve_their_public_errno_contract() {
    let cases = [
        (Error::Fs(libc::EAGAIN), Errno::EAGAIN),
        (Error::Config("invalid option".into()), Errno::EINVAL),
        (Error::Unsupported("operation".into()), Errno::EOPNOTSUPP),
        (
            Error::Budget {
                requested: 2,
                limit: 1,
            },
            Errno::ENOMEM,
        ),
        (Error::Timeout("GET"), Errno::ETIMEDOUT),
        (Error::Cancelled, Errno::EINTR),
        (
            Error::InvalidResponse("missing generation".into()),
            Errno::EIO,
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(errno(error), expected);
    }
}

#[test]
fn s3_missing_access_generation_and_unknown_errors_remain_distinct() {
    for (code, expected) in [
        ("NoSuchKey", Errno::ENOENT),
        ("NotFound", Errno::ENOENT),
        ("404", Errno::ENOENT),
        ("AccessDenied", Errno::EACCES),
        ("InvalidAccessKeyId", Errno::EACCES),
        ("SignatureDoesNotMatch", Errno::EACCES),
        ("PreconditionFailed", Errno::ESTALE),
        ("InternalError", Errno::EIO),
    ] {
        assert_eq!(
            errno(Error::Backend {
                operation: "GET",
                code: code.into(),
                message: "dependency failure".into()
            }),
            expected,
            "S3 code {code}"
        );
    }
}
