//! The platform's errno, signal and filesystem constants.
//!
//! Every value comes from `libc`, so it is the number THIS build's platform
//! really uses. That matters: `EADDRINUSE` is 48 on macOS and 98 on Linux,
//! `SIGUSR1` is 30 and 10, `SIGSTOP` is 17 and 19. The signal table used to be
//! a hardcoded list of macOS numbers, so `os.constants.signals.SIGUSR1` was
//! wrong on every Linux host and `errno` was missing entirely.
//!
//! These feed `os.constants`, `fs.constants` and the legacy flat
//! `require('constants')`, which is the union of all three.

use crate::host::with_host;
use fusevm::Value;
use indexmap::IndexMap;

/// `(name, value)` for every errno node exposes. A name absent on this platform
/// is simply not listed, exactly as node's own table is platform-shaped.
pub fn errno() -> Vec<(&'static str, i64)> {
    let mut v: Vec<(&'static str, i64)> = vec![
        ("E2BIG", libc::E2BIG as i64),
        ("EACCES", libc::EACCES as i64),
        ("EADDRINUSE", libc::EADDRINUSE as i64),
        ("EADDRNOTAVAIL", libc::EADDRNOTAVAIL as i64),
        ("EAFNOSUPPORT", libc::EAFNOSUPPORT as i64),
        ("EAGAIN", libc::EAGAIN as i64),
        ("EALREADY", libc::EALREADY as i64),
        ("EBADF", libc::EBADF as i64),
        ("EBADMSG", libc::EBADMSG as i64),
        ("EBUSY", libc::EBUSY as i64),
        ("ECANCELED", libc::ECANCELED as i64),
        ("ECHILD", libc::ECHILD as i64),
        ("ECONNABORTED", libc::ECONNABORTED as i64),
        ("ECONNREFUSED", libc::ECONNREFUSED as i64),
        ("ECONNRESET", libc::ECONNRESET as i64),
        ("EDEADLK", libc::EDEADLK as i64),
        ("EDESTADDRREQ", libc::EDESTADDRREQ as i64),
        ("EDOM", libc::EDOM as i64),
        ("EDQUOT", libc::EDQUOT as i64),
        ("EEXIST", libc::EEXIST as i64),
        ("EFAULT", libc::EFAULT as i64),
        ("EFBIG", libc::EFBIG as i64),
        ("EHOSTUNREACH", libc::EHOSTUNREACH as i64),
        ("EIDRM", libc::EIDRM as i64),
        ("EILSEQ", libc::EILSEQ as i64),
        ("EINPROGRESS", libc::EINPROGRESS as i64),
        ("EINTR", libc::EINTR as i64),
        ("EINVAL", libc::EINVAL as i64),
        ("EIO", libc::EIO as i64),
        ("EISCONN", libc::EISCONN as i64),
        ("EISDIR", libc::EISDIR as i64),
        ("ELOOP", libc::ELOOP as i64),
        ("EMFILE", libc::EMFILE as i64),
        ("EMLINK", libc::EMLINK as i64),
        ("EMSGSIZE", libc::EMSGSIZE as i64),
        ("EMULTIHOP", libc::EMULTIHOP as i64),
        ("ENAMETOOLONG", libc::ENAMETOOLONG as i64),
        ("ENETDOWN", libc::ENETDOWN as i64),
        ("ENETRESET", libc::ENETRESET as i64),
        ("ENETUNREACH", libc::ENETUNREACH as i64),
        ("ENFILE", libc::ENFILE as i64),
        ("ENOBUFS", libc::ENOBUFS as i64),
        ("ENODATA", libc::ENODATA as i64),
        ("ENODEV", libc::ENODEV as i64),
        ("ENOENT", libc::ENOENT as i64),
        ("ENOEXEC", libc::ENOEXEC as i64),
        ("ENOLCK", libc::ENOLCK as i64),
        ("ENOLINK", libc::ENOLINK as i64),
        ("ENOMEM", libc::ENOMEM as i64),
        ("ENOMSG", libc::ENOMSG as i64),
        ("ENOPROTOOPT", libc::ENOPROTOOPT as i64),
        ("ENOSPC", libc::ENOSPC as i64),
        ("ENOSR", libc::ENOSR as i64),
        ("ENOSTR", libc::ENOSTR as i64),
        ("ENOSYS", libc::ENOSYS as i64),
        ("ENOTCONN", libc::ENOTCONN as i64),
        ("ENOTDIR", libc::ENOTDIR as i64),
        ("ENOTEMPTY", libc::ENOTEMPTY as i64),
        ("ENOTSOCK", libc::ENOTSOCK as i64),
        ("ENOTSUP", libc::ENOTSUP as i64),
        ("ENOTTY", libc::ENOTTY as i64),
        ("ENXIO", libc::ENXIO as i64),
        ("EOPNOTSUPP", libc::EOPNOTSUPP as i64),
        ("EOVERFLOW", libc::EOVERFLOW as i64),
        ("EPERM", libc::EPERM as i64),
        ("EPIPE", libc::EPIPE as i64),
        ("EPROTO", libc::EPROTO as i64),
        ("EPROTONOSUPPORT", libc::EPROTONOSUPPORT as i64),
        ("EPROTOTYPE", libc::EPROTOTYPE as i64),
        ("ERANGE", libc::ERANGE as i64),
        ("EROFS", libc::EROFS as i64),
        ("ESPIPE", libc::ESPIPE as i64),
        ("ESRCH", libc::ESRCH as i64),
        ("ESTALE", libc::ESTALE as i64),
        ("ETIME", libc::ETIME as i64),
        ("ETIMEDOUT", libc::ETIMEDOUT as i64),
        ("ETXTBSY", libc::ETXTBSY as i64),
        ("EWOULDBLOCK", libc::EWOULDBLOCK as i64),
        ("EXDEV", libc::EXDEV as i64),
    ];
    v.sort_by_key(|(k, _)| *k);
    v
}

/// `(name, number)` for every signal node exposes.
pub fn signals() -> Vec<(&'static str, i64)> {
    let mut v: Vec<(&'static str, i64)> = vec![
        ("SIGHUP", libc::SIGHUP as i64),
        ("SIGINT", libc::SIGINT as i64),
        ("SIGQUIT", libc::SIGQUIT as i64),
        ("SIGILL", libc::SIGILL as i64),
        ("SIGTRAP", libc::SIGTRAP as i64),
        ("SIGABRT", libc::SIGABRT as i64),
        ("SIGIOT", libc::SIGABRT as i64),
        ("SIGBUS", libc::SIGBUS as i64),
        ("SIGFPE", libc::SIGFPE as i64),
        ("SIGKILL", libc::SIGKILL as i64),
        ("SIGUSR1", libc::SIGUSR1 as i64),
        ("SIGSEGV", libc::SIGSEGV as i64),
        ("SIGUSR2", libc::SIGUSR2 as i64),
        ("SIGPIPE", libc::SIGPIPE as i64),
        ("SIGALRM", libc::SIGALRM as i64),
        ("SIGTERM", libc::SIGTERM as i64),
        ("SIGCHLD", libc::SIGCHLD as i64),
        ("SIGCONT", libc::SIGCONT as i64),
        ("SIGSTOP", libc::SIGSTOP as i64),
        ("SIGTSTP", libc::SIGTSTP as i64),
        ("SIGTTIN", libc::SIGTTIN as i64),
        ("SIGTTOU", libc::SIGTTOU as i64),
        ("SIGURG", libc::SIGURG as i64),
        ("SIGXCPU", libc::SIGXCPU as i64),
        ("SIGXFSZ", libc::SIGXFSZ as i64),
        ("SIGVTALRM", libc::SIGVTALRM as i64),
        ("SIGPROF", libc::SIGPROF as i64),
        ("SIGWINCH", libc::SIGWINCH as i64),
        ("SIGIO", libc::SIGIO as i64),
        ("SIGSYS", libc::SIGSYS as i64),
    ];
    // `SIGINFO` is a BSD signal; Linux has no such number.
    #[cfg(target_os = "macos")]
    v.push(("SIGINFO", libc::SIGINFO as i64));
    v.sort_by_key(|(k, _)| *k);
    v
}

/// `(name, value)` for `fs.constants` — open flags, access modes, file-type
/// bits, permission bits and the copyfile flags.
pub fn fs() -> Vec<(&'static str, i64)> {
    vec![
        ("UV_FS_SYMLINK_DIR", 1),
        ("UV_FS_SYMLINK_JUNCTION", 2),
        ("O_RDONLY", libc::O_RDONLY as i64),
        ("O_WRONLY", libc::O_WRONLY as i64),
        ("O_RDWR", libc::O_RDWR as i64),
        ("UV_DIRENT_UNKNOWN", 0),
        ("UV_DIRENT_FILE", 1),
        ("UV_DIRENT_DIR", 2),
        ("UV_DIRENT_LINK", 3),
        ("UV_DIRENT_FIFO", 4),
        ("UV_DIRENT_SOCKET", 5),
        ("UV_DIRENT_CHAR", 6),
        ("UV_DIRENT_BLOCK", 7),
        ("S_IFMT", libc::S_IFMT as i64),
        ("S_IFREG", libc::S_IFREG as i64),
        ("S_IFDIR", libc::S_IFDIR as i64),
        ("S_IFCHR", libc::S_IFCHR as i64),
        ("S_IFBLK", libc::S_IFBLK as i64),
        ("S_IFIFO", libc::S_IFIFO as i64),
        ("S_IFLNK", libc::S_IFLNK as i64),
        ("S_IFSOCK", libc::S_IFSOCK as i64),
        ("O_CREAT", libc::O_CREAT as i64),
        ("O_EXCL", libc::O_EXCL as i64),
        ("O_NOCTTY", libc::O_NOCTTY as i64),
        ("O_TRUNC", libc::O_TRUNC as i64),
        ("O_APPEND", libc::O_APPEND as i64),
        ("O_DIRECTORY", libc::O_DIRECTORY as i64),
        ("O_NOFOLLOW", libc::O_NOFOLLOW as i64),
        ("O_SYNC", libc::O_SYNC as i64),
        ("O_DSYNC", libc::O_DSYNC as i64),
        ("O_NONBLOCK", libc::O_NONBLOCK as i64),
        ("S_IRWXU", libc::S_IRWXU as i64),
        ("S_IRUSR", libc::S_IRUSR as i64),
        ("S_IWUSR", libc::S_IWUSR as i64),
        ("S_IXUSR", libc::S_IXUSR as i64),
        ("S_IRWXG", libc::S_IRWXG as i64),
        ("S_IRGRP", libc::S_IRGRP as i64),
        ("S_IWGRP", libc::S_IWGRP as i64),
        ("S_IXGRP", libc::S_IXGRP as i64),
        ("S_IRWXO", libc::S_IRWXO as i64),
        ("S_IROTH", libc::S_IROTH as i64),
        ("S_IWOTH", libc::S_IWOTH as i64),
        ("S_IXOTH", libc::S_IXOTH as i64),
        ("F_OK", libc::F_OK as i64),
        ("R_OK", libc::R_OK as i64),
        ("W_OK", libc::W_OK as i64),
        ("X_OK", libc::X_OK as i64),
        ("UV_FS_COPYFILE_EXCL", 1),
        ("COPYFILE_EXCL", 1),
        ("UV_FS_COPYFILE_FICLONE", 2),
        ("COPYFILE_FICLONE", 2),
        ("UV_FS_COPYFILE_FICLONE_FORCE", 4),
        ("COPYFILE_FICLONE_FORCE", 4),
    ]
}

/// `os.constants.dlopen`.
pub fn dlopen() -> Vec<(&'static str, i64)> {
    vec![
        ("RTLD_LAZY", libc::RTLD_LAZY as i64),
        ("RTLD_NOW", libc::RTLD_NOW as i64),
        ("RTLD_GLOBAL", libc::RTLD_GLOBAL as i64),
        ("RTLD_LOCAL", libc::RTLD_LOCAL as i64),
    ]
}

/// `os.constants.priority` — libuv's own scale, not a platform one.
pub fn priority() -> Vec<(&'static str, i64)> {
    vec![
        ("PRIORITY_LOW", 19),
        ("PRIORITY_BELOW_NORMAL", 10),
        ("PRIORITY_NORMAL", 0),
        ("PRIORITY_ABOVE_NORMAL", -7),
        ("PRIORITY_HIGH", -14),
        ("PRIORITY_HIGHEST", -20),
    ]
}

/// `crypto.constants` — OpenSSL's own defines, which are the same numbers
/// everywhere, plus the two padding constants libraries actually pass.
pub fn crypto() -> Vec<(&'static str, i64)> {
    vec![
        ("SSL_OP_ALL", 0x80000BFF),
        ("SSL_OP_ALLOW_NO_DHE_KEX", 0x400),
        ("SSL_OP_ALLOW_UNSAFE_LEGACY_RENEGOTIATION", 0x40000),
        ("SSL_OP_CIPHER_SERVER_PREFERENCE", 0x400000),
        ("SSL_OP_CISCO_ANYCONNECT", 0x8000),
        ("SSL_OP_COOKIE_EXCHANGE", 0x2000),
        ("SSL_OP_CRYPTOPRO_TLSEXT_BUG", 0x80000000),
        ("SSL_OP_DONT_INSERT_EMPTY_FRAGMENTS", 0x800),
        ("SSL_OP_LEGACY_SERVER_CONNECT", 0x4),
        ("SSL_OP_NO_COMPRESSION", 0x20000),
        ("SSL_OP_NO_ENCRYPT_THEN_MAC", 0x80000),
        ("SSL_OP_NO_QUERY_MTU", 0x1000),
        ("SSL_OP_NO_RENEGOTIATION", 0x40000000),
        ("SSL_OP_NO_SESSION_RESUMPTION_ON_RENEGOTIATION", 0x10000),
        ("SSL_OP_NO_SSLv2", 0x0),
        ("SSL_OP_NO_SSLv3", 0x2000000),
        ("SSL_OP_NO_TLSv1", 0x4000000),
        ("SSL_OP_NO_TLSv1_1", 0x10000000),
        ("SSL_OP_NO_TLSv1_2", 0x8000000),
        ("SSL_OP_NO_TLSv1_3", 0x20000000),
        ("SSL_OP_PRIORITIZE_CHACHA", 0x200000),
        ("SSL_OP_TLS_ROLLBACK_BUG", 0x800000),
        ("ENGINE_METHOD_RSA", 1),
        ("ENGINE_METHOD_DSA", 2),
        ("ENGINE_METHOD_DH", 4),
        ("ENGINE_METHOD_RAND", 8),
        ("ENGINE_METHOD_EC", 2048),
        ("ENGINE_METHOD_CIPHERS", 64),
        ("ENGINE_METHOD_DIGESTS", 128),
        ("ENGINE_METHOD_PKEY_METHS", 512),
        ("ENGINE_METHOD_PKEY_ASN1_METHS", 1024),
        ("ENGINE_METHOD_ALL", 65535),
        ("ENGINE_METHOD_NONE", 0),
        ("DH_CHECK_P_NOT_SAFE_PRIME", 2),
        ("DH_CHECK_P_NOT_PRIME", 1),
        ("DH_UNABLE_TO_CHECK_GENERATOR", 4),
        ("DH_NOT_SUITABLE_GENERATOR", 8),
        ("RSA_PKCS1_PADDING", 1),
        ("RSA_NO_PADDING", 3),
        ("RSA_PKCS1_OAEP_PADDING", 4),
        ("RSA_X931_PADDING", 5),
        ("RSA_PKCS1_PSS_PADDING", 6),
        ("RSA_PSS_SALTLEN_DIGEST", -1),
        ("RSA_PSS_SALTLEN_MAX_SIGN", -2),
        ("RSA_PSS_SALTLEN_AUTO", -2),
        ("POINT_CONVERSION_COMPRESSED", 2),
        ("POINT_CONVERSION_UNCOMPRESSED", 4),
        ("POINT_CONVERSION_HYBRID", 6),
        ("TLS1_VERSION", 769),
        ("TLS1_1_VERSION", 770),
        ("TLS1_2_VERSION", 771),
        ("TLS1_3_VERSION", 772),
        ("OPENSSL_VERSION_NUMBER", 0x30000000),
        ("defaultCoreCipherList", 0),
        ("defaultCipherList", 0),
    ]
}

/// A `{name: value}` object from a constant table.
pub fn object(entries: &[(&'static str, i64)]) -> Value {
    with_host(|h| {
        let mut m: IndexMap<String, Value> = IndexMap::new();
        for (k, v) in entries {
            m.insert((*k).to_string(), Value::Float(*v as f64));
        }
        h.new_object(m)
    })
}

/// The legacy flat `require('constants')`: the union of the dlopen, errno,
/// signal, fs and crypto tables, in the order node lists them. A name defined
/// by more than one table keeps the FIRST definition, as node's does.
pub fn flat() -> Vec<(&'static str, i64)> {
    let mut out: Vec<(&'static str, i64)> = Vec::new();
    for table in [dlopen(), errno(), signals(), fs(), crypto()] {
        for (k, v) in table {
            if !out.iter().any(|(n, _)| *n == k) {
                out.push((k, v));
            }
        }
    }
    out
}

/// One member of the flat table, for a `require('constants').NAME` read.
pub fn flat_lookup(name: &str) -> Option<Value> {
    flat()
        .into_iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| Value::Float(v as f64))
}
