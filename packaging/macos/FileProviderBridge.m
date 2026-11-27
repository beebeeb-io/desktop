#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>
#import <FileProvider/FileProvider.h>
#import <dispatch/dispatch.h>
#include <dirent.h>
#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <sys/stat.h>

static NSString *BeebeebDomainIdentifier = @"io.beebeeb.app.domain";
// Task 1698 part 3 (1696 G8): the domain's display name is "Beebeeb" — the
// app's name and what the system DB + Finder sidebar already show. The old
// "Drive" was the zombie domain's residue (1696 forensics: we registered
// "Drive" while the system showed "Beebeeb"). Which side wins on an existing
// registration is device-verifiable only (addDomain updates the stored
// domain); the registered truth is now the app's own name.
//
// Correction — 2026-10-06 (spec docs/specs/2026-10-06-macos-finder-setup-reconciler.md §2):
// the forensics line above read the ZOMBIE's name. "Beebeeb" was the display name of the
// orphan `io.beebeeb.desktop.FileProvider` domain, whose folder is
// ~/Library/CloudStorage/Beebeeb-Beebeeb. Renaming ours to "Beebeeb" made ours ask for that
// same folder, and every add then failed with the folder collision. The name stays "Beebeeb"
// (the orphan is cleared by spec §11's one-off helper); this note keeps the wrong claim visible.
static NSString *BeebeebDomainDisplayName = @"Beebeeb";

// Spec 2026-10-06 §6.1: every failure crosses the FFI as (domain, code, message) plus the
// NSUnderlyingErrorKey error's domain and code (the folder collision may arrive as an
// underlying POSIX EEXIST). The 0.8.x Rust side mirrored it as `BeebeebFpErrorC` (the Tauri shell's
// `macos_file_provider.rs`, tag desktop-v0.8.25; native/beebeeb-drive-rune/docs/macos/file-provider-bridge.md).
typedef struct {
    int64_t code;
    int64_t underlying_code;
    int32_t has_underlying;
    char domain[128];
    char underlying_domain[128];
    char message[1024];
} BeebeebFpError;

// Layout pins. The Rust side asserts the same numbers at compile time, and its
// `bridge_error_tests` compare the exported `beebeeb_fp_test_error_*` values at runtime.
_Static_assert(sizeof(BeebeebFpError) == 1304, "BeebeebFpError changed: update BeebeebFpErrorC in macos_file_provider.rs");
_Static_assert(_Alignof(BeebeebFpError) == 8, "BeebeebFpError alignment changed: update BeebeebFpErrorC");
_Static_assert(offsetof(BeebeebFpError, code) == 0, "BeebeebFpError.code moved");
_Static_assert(offsetof(BeebeebFpError, underlying_code) == 8, "BeebeebFpError.underlying_code moved");
_Static_assert(offsetof(BeebeebFpError, has_underlying) == 16, "BeebeebFpError.has_underlying moved");
_Static_assert(offsetof(BeebeebFpError, domain) == 20, "BeebeebFpError.domain moved");
_Static_assert(offsetof(BeebeebFpError, underlying_domain) == 148, "BeebeebFpError.underlying_domain moved");
_Static_assert(offsetof(BeebeebFpError, message) == 276, "BeebeebFpError.message moved");

// Errors this bridge synthesizes when no NSError exists. Codes mirrored by
// `finder_setup::error::bridge_code` in Rust; `beebeeb_fp_test_bridge_codes` exports them so a
// Rust test pins the two sets against each other.
static NSString *const BeebeebBridgeErrorDomain = @"io.beebeeb.bridge";
enum {
    BeebeebBridgeManagerUnavailable = 1,
    BeebeebBridgeStabilizationTimeout = 2,
    BeebeebBridgeResolveUrlTimeout = 3,
    BeebeebBridgeSignalTimeout = 4,
    BeebeebBridgeNoIdentifier = 5,
    // Task 1885: NSWorkspace did not answer an open within BeebeebOpenTimeoutSeconds.
    BeebeebBridgeOpenTimeout = 6,
    // A URL handed to beebeeb_open_url that does not parse, or is a file URL.
    BeebeebBridgeInvalidURL = 7,
    // The Finder location macOS returned for an item is not on disk.
    BeebeebBridgeItemMissing = 8,
};

// Every write into a caller's buffer goes through here: bounded by the buffer's own length and
// always NUL-terminated (strlcpy copies at most buffer_len - 1 bytes, then terminates).
static void BeebeebCopyString(NSString *value, char *buffer, size_t buffer_len) {
    if (buffer == NULL || buffer_len == 0) {
        return;
    }
    const char *utf8 = [value UTF8String];
    strlcpy(buffer, utf8 ?: "", buffer_len);
}

static void BeebeebFillError(NSError *error, BeebeebFpError *out) {
    if (out == NULL) {
        return;
    }
    memset(out, 0, sizeof(*out));
    out->code = (int64_t)error.code;
    BeebeebCopyString(error.domain ?: @"unknown-domain", out->domain, sizeof(out->domain));
    BeebeebCopyString(error.localizedDescription ?: @"unknown File Provider error", out->message, sizeof(out->message));
    NSError *underlying = error.userInfo[NSUnderlyingErrorKey];
    if ([underlying isKindOfClass:[NSError class]]) {
        out->has_underlying = 1;
        out->underlying_code = (int64_t)underlying.code;
        BeebeebCopyString(underlying.domain ?: @"unknown-domain", out->underlying_domain, sizeof(out->underlying_domain));
    }
}

static void BeebeebFillBridgeError(int64_t code, NSString *message, BeebeebFpError *out) {
    if (out == NULL) {
        return;
    }
    memset(out, 0, sizeof(*out));
    out->code = code;
    BeebeebCopyString(BeebeebBridgeErrorDomain, out->domain, sizeof(out->domain));
    BeebeebCopyString(message, out->message, sizeof(out->message));
}

static NSFileProviderDomain *BeebeebDomain(void) {
    NSFileProviderDomain *domain = [[NSFileProviderDomain alloc] initWithIdentifier:BeebeebDomainIdentifier
                                                            displayName:BeebeebDomainDisplayName];
    // Task 1698 part 2 (ruling: full trash sync): macOS 13+ surfaces the
    // system trash container as Finder's Trash when this is YES. It IS the
    // default, but it is set explicitly so the ruling is visible in code and
    // survives any future default change.
    if (@available(macOS 13.0, *)) {
        domain.supportsSyncingTrash = YES;
    }
    return domain;
}

// The system's live registry. Returns 0 and fills `*found` (nil when not registered), or -1.
static int BeebeebFindOurDomain(NSFileProviderDomain **found, BeebeebFpError *out_error) {
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSArray<NSFileProviderDomain *> *found_domains = nil;
    __block NSError *found_error = nil;
    [NSFileProviderManager getDomainsWithCompletionHandler:^(NSArray<NSFileProviderDomain *> *domains, NSError *error) {
        found_domains = domains;
        found_error = error;
        dispatch_semaphore_signal(semaphore);
    }];
    dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
    if (found_error != nil) {
        BeebeebFillError(found_error, out_error);
        return -1;
    }
    *found = nil;
    for (NSFileProviderDomain *domain in found_domains) {
        if ([domain.identifier isEqualToString:BeebeebDomainIdentifier]) {
            *found = domain;
            break;
        }
    }
    return 0;
}

static int BeebeebWaitForDomainReady(NSFileProviderDomain *domain, double timeout_seconds, BeebeebFpError *out_error) {
    NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:domain];
    if (manager == nil) {
        BeebeebFillBridgeError(BeebeebBridgeManagerUnavailable,
                               @"File Provider manager is unavailable for the Beebeeb domain", out_error);
        return -1;
    }
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSError *found_error = nil;
    [manager waitForStabilizationWithCompletionHandler:^(NSError *error) {
        found_error = error;
        dispatch_semaphore_signal(semaphore);
    }];
    int64_t timeout_nanos = (int64_t)(timeout_seconds * (double)NSEC_PER_SEC);
    long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, timeout_nanos));
    if (wait_result != 0) {
        BeebeebFillBridgeError(BeebeebBridgeStabilizationTimeout,
                               @"Timed out waiting for the Beebeeb File Provider domain to become available", out_error);
        return -1;
    }
    if (found_error != nil) {
        BeebeebFillError(found_error, out_error);
        return -1;
    }
    return 0;
}

// Task 1882 round 2 (spec docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md §4, §5):
// what the folder macOS reported after a removal holds. A reported URL does not mean anything was
// kept (device K-F2: an empty domain's removal reported a folder that never existed), so the
// person is told only when the folder exists. The 0.8.x Rust side (`finder_removal.rs`,
// `settle_kept_state`, tag desktop-v0.8.25) looked again at a folder that reads as missing, because
// macOS may make or fill it after this completion handler runs; an existing folder is kept, empty
// or not (re-review D1). The same numbers as its `KEPT_*`;
// crates/beebeeb-engine/tests/file_provider_source_pins.rs pins them.
enum {
    BeebeebKeptNoneReported = 0,
    BeebeebKeptMissing = 1,
    BeebeebKeptEmpty = 2,
    BeebeebKeptHasEntries = 3,
    BeebeebKeptUnchecked = 4,
    BeebeebKeptNoPath = 5,
};

// Checks the folder on the URL macOS returned. The header does not say that URL is
// security-scoped (contrast NSFileProviderManager.h:103, :120-121), and a path string never
// carries a scope (NSURL.h:103), so the check runs here, on the URL itself. The App Sandbox
// allows `stat` everywhere (application.sb:496, `(allow file-read-metadata)`); listing needs the
// URL's extension. If the listing is refused, the folder is reported unchecked: it exists, so the
// person is pointed at a real folder, never at nothing. It stops at the first entry and keeps no
// name; it never opens a file.
static int BeebeebKeptFolderState(NSURL *location) {
    if (location == nil) {
        return BeebeebKeptNoneReported;
    }
    if (location.path.length == 0) {
        return BeebeebKeptNoPath;
    }
    const char *fs_path = location.fileSystemRepresentation;
    if (fs_path == NULL || fs_path[0] == '\0') {
        return BeebeebKeptNoPath;
    }

    BOOL scoped = [location startAccessingSecurityScopedResource];
    int state;
    struct stat info;
    if (lstat(fs_path, &info) != 0) {
        state = (errno == ENOENT || errno == ENOTDIR) ? BeebeebKeptMissing : BeebeebKeptUnchecked;
    } else if (!S_ISDIR(info.st_mode)) {
        // macOS kept a single item rather than a folder: that is something kept.
        state = BeebeebKeptHasEntries;
    } else {
        DIR *dir = opendir(fs_path);
        if (dir == NULL) {
            state = (errno == ENOENT || errno == ENOTDIR) ? BeebeebKeptMissing : BeebeebKeptUnchecked;
        } else {
            state = BeebeebKeptEmpty;
            errno = 0;
            struct dirent *entry;
            while ((entry = readdir(dir)) != NULL) {
                if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
                    continue;
                }
                state = BeebeebKeptHasEntries;
                break;
            }
            if (entry == NULL && errno != 0) {
                state = BeebeebKeptUnchecked;
            }
            closedir(dir);
        }
    }
    if (scoped) {
        [location stopAccessingSecurityScopedResource];
    }
    return state;
}

// Task 1882 (P0, spec docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md): EVERY
// domain removal keeps the files that never reached the server. The plain
// `removeDomain:completionHandler:` deleted them with the domain (three Finder-created files on a
// device, task 1873). `NSFileProviderDomainRemovalModePreserveDirtyUserData` is macOS 12.0+
// (NSFileProviderManager.h:26, :239; NSFileProviderDefines.h:25); this bridge is built for 14.0
// (the 0.8.x build script; its compile line is carried in
// native/beebeeb-drive-rune/docs/macos/file-provider-bridge.md), so there is no fallback to the
// remove-all form, on purpose. crates/beebeeb-engine/tests/file_provider_source_pins.rs pins every
// removal in the repo to this mode.
//
// Returns: 0 = removed (`kept_state` says what the reported folder holds; `location_buffer` holds
// its path whenever it has one); -1 = error (`out_error` set, spec 2026-10-06 §6.1; `kept_state`
// and `location_buffer` are still filled, review M2).
static int BeebeebRemoveDomainKeepingUnsynced(NSFileProviderDomain *domain,
                                              char *location_buffer,
                                              unsigned long location_buffer_len,
                                              int *kept_state,
                                              BeebeebFpError *out_error) {
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSURL *found_location = nil;
    __block NSError *found_error = nil;
    if (kept_state != NULL) {
        *kept_state = BeebeebKeptNoneReported;
    }

    [NSFileProviderManager removeDomain:domain
                                   mode:NSFileProviderDomainRemovalModePreserveDirtyUserData
                      completionHandler:^(NSURL *preservedLocation, NSError *error) {
        found_location = preservedLocation;
        found_error = error;
        dispatch_semaphore_signal(semaphore);
    }];
    dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);

    // Review M2: the folder is checked first, so a reply that carries both an error and a folder
    // still reaches the person with that folder.
    int state = BeebeebKeptFolderState(found_location);
    if (kept_state != NULL) {
        *kept_state = state;
    }
    NSString *path = found_location.path;
    if (path.length > 0) {
        BeebeebCopyString(path, location_buffer, location_buffer_len);
    }
    if (found_error != nil) {
        BeebeebFillError(found_error, out_error);
        return -1;
    }
    return 0;
}

// Runs the same folder check on a file URL built from `path`. Rust calls it to look again at a
// folder that read as missing (re-review D1: macos_file_provider.rs `kept_state_for_path`; `stat`
// needs no scope, so a path is enough), and the tests call it to test the check on a real disk.
int beebeeb_fp_kept_folder_state_for_path(const char *path) {
    @autoreleasepool {
        if (path == NULL) {
            return BeebeebKeptFolderState(nil);
        }
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        return BeebeebKeptFolderState(url);
    }
}

// Task 1885: the Finder location of an item, as the URL macOS returned (never a path string).
//
// `getUserVisibleURLForItemIdentifier:completionHandler:` returns a SECURITY-SCOPED URL: the header
// says so and says what to do with it ("the caller must call `-[NSURL startAccessingSecurityScopedResource]`
// on the returned URL", NSFileProviderManager.h:120-121; "The returned URL grants read-write access to the
// user visible location", :123). A path made from it carries no scope (NSURL.h:103), and neither does a
// child process: `/usr/bin/open <path>` inherited the App Sandbox without the extension, and LaunchServices
// refused the CloudStorage folder with -54 while `spawn()` had already answered Ok. So the URL stays a URL
// and is opened in THIS process. Callers use `*status`: 1 = a URL, 0 = macOS reported none, -1 = error.
static NSURL *BeebeebUserVisibleURL(NSFileProviderItemIdentifier identifier, int *status, BeebeebFpError *out_error) {
    *status = -1;
    NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:BeebeebDomain()];
    if (manager == nil) {
        BeebeebFillBridgeError(BeebeebBridgeManagerUnavailable,
                               @"File Provider manager is unavailable for the Beebeeb domain", out_error);
        return nil;
    }
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSURL *found_url = nil;
    __block NSError *found_error = nil;
    [manager getUserVisibleURLForItemIdentifier:identifier
                              completionHandler:^(NSURL *url, NSError *error) {
        found_url = url;
        found_error = error;
        dispatch_semaphore_signal(semaphore);
    }];
    long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
    if (wait_result != 0) {
        BeebeebFillBridgeError(BeebeebBridgeResolveUrlTimeout, @"Timed out resolving the Beebeeb Finder location", out_error);
        return nil;
    }
    if (found_error != nil) {
        BeebeebFillError(found_error, out_error);
        return nil;
    }
    *status = (found_url == nil) ? 0 : 1;
    return found_url;
}

// Task 1885 fix round 1 (I1): the resolve is a File Provider call and runs under Rust's bridge gate; the
// open is NOT (NSWorkspace is not a File Provider call, and it can stall for seconds: a sign-out's removal
// waits only 3 s for the gate and is never retried). So the scoped URL crosses the FFI as a retained,
// opaque handle (`CFBridgingRetain`), the gate is released, and the open (or reveal) takes the handle
// back (`CFBridgingRelease`) and always releases it. `BeebeebLiveHandles` counts handles in flight so a
// test can see none leaks.
static volatile int64_t BeebeebLiveHandles = 0;

static void *BeebeebRetainHandle(NSURL *url) {
    __atomic_add_fetch(&BeebeebLiveHandles, 1, __ATOMIC_SEQ_CST);
    return (void *)CFBridgingRetain(url);
}

static NSURL *BeebeebTakeHandle(void *handle) {
    if (handle == NULL) {
        return nil;
    }
    __atomic_sub_fetch(&BeebeebLiveHandles, 1, __ATOMIC_SEQ_CST);
    return (NSURL *)CFBridgingRelease(handle);
}

// A security scope that is released exactly once, by whichever of its owners comes first: the open's
// completion handler, or the safety timer below. `startAccessingSecurityScopedResource` is counted by the
// system, so a second stop would take a reference somebody else holds.
@interface BeebeebScope : NSObject
- (instancetype)initWithURL:(NSURL *)url;
- (void)stop;
@end

@implementation BeebeebScope {
    NSURL *_url;
    BOOL _active;
}

- (instancetype)initWithURL:(NSURL *)url {
    if ((self = [super init])) {
        _url = url;
        _active = url.isFileURL ? [url startAccessingSecurityScopedResource] : NO;
    }
    return self;
}

- (void)stop {
    @synchronized(self) {
        if (_active) {
            _active = NO;
            [_url stopAccessingSecurityScopedResource];
        }
    }
}
@end

// How long an open waits for LaunchServices. A person clicked and waits, so this is short: LaunchServices
// answers a folder open in milliseconds, and silence is reported as a failure, never as a success. The open
// holds no bridge gate (see above), so a stall here delays nothing but this one call.
static const int64_t BeebeebOpenTimeoutSeconds = 5;

// The longest a scope outlives its open when the completion handler never comes (m3). `NSWorkspace` gives
// no way to cancel an open, so after the timeout above the answer may still arrive; if it never does, this
// timer is what releases the scope, and the handler's later call (if any) finds it already stopped.
static const int64_t BeebeebScopeSafetySeconds = 60;

// Task 1885: open `url` through NSWorkspace and WAIT for the answer. 0 = LaunchServices opened it,
// -1 = it refused (its NSError crosses as structured, like every File Provider error) or did not answer
// in time (BeebeebBridgeOpenTimeout). Nothing here is fire-and-forget: a `spawn()` that answers before
// the OS has is how "Open in Finder" reported success for an open that failed.
//
// A file URL keeps its scope until LaunchServices answers, which is the last thing that needs it. After
// the timeout the open is not cancelled (there is no API for that): a success that arrives later still
// opens the window, after the caller has already been told it failed (a false error, bounded to a stall
// longer than the limit above).
//
// `promptsUserIfNeeded = NO`: the completion handler "will not be invoked until the user dismisses any
// such UI" (NSWorkspace.h:155-158), and a wait that a dialog can hold back is not a wait this bridge can bound.
//
// dm-84 (83's rule for the page's buttons, now for the Files list's double-click too): RC15's double-click on an .eps
// started Illustrator and the list said "Couldn't open", because a big app answers after the 5 s. An app the system
// handed the file to opened it, whatever error it also answered, and so did the file's app when it was not running
// before this open and runs when the wait ends (it is still starting with the file). Only an error with no app, or a
// silence with nothing started, is a refusal. A swallowed error is logged with its domain and code.
// The rule above as a pure function, so a test holds it (`engine_host/mod.rs`): 0 = opened, -1 = refused.
// `answered`: the completion handler came within the wait; `handed`: it named an app; `has_error`: it carried an
// error; `ran_before`/`runs_now`: the file's app ran before the open / runs when the wait ended.
int beebeeb_open_outcome(int answered, int handed, int has_error, int ran_before, int runs_now) {
    if (!answered) {
        return (!ran_before && runs_now) ? 0 : -1;
    }
    return (!has_error || handed) ? 0 : -1;
}

static int BeebeebOpenURLAndWait(NSURL *url, BeebeebFpError *out_error) {
    BeebeebScope *scope = [[BeebeebScope alloc] initWithURL:url];
    NSWorkspaceOpenConfiguration *configuration = [NSWorkspaceOpenConfiguration configuration];
    configuration.promptsUserIfNeeded = NO;
    configuration.addsToRecentItems = NO;
    // The app that opens this kind of file, and whether it ran before this open (a file URL only).
    NSURL *app_url = url.isFileURL ? [[NSWorkspace sharedWorkspace] URLForApplicationToOpenURL:url] : nil;
    NSString *app_id = app_url == nil ? nil : [NSBundle bundleWithURL:app_url].bundleIdentifier;
    BOOL ran_before = app_id != nil && [NSRunningApplication runningApplicationsWithBundleIdentifier:app_id].count > 0;
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSError *found_error = nil;
    __block BOOL handed = NO;
    [[NSWorkspace sharedWorkspace] openURL:url configuration:configuration
                         completionHandler:^(NSRunningApplication *application, NSError *error) {
        found_error = error;
        handed = application != nil;
        [scope stop];
        dispatch_semaphore_signal(semaphore);
    }];
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, BeebeebScopeSafetySeconds * NSEC_PER_SEC),
                   dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
        [scope stop];
    });
    long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, BeebeebOpenTimeoutSeconds * NSEC_PER_SEC));
    if (wait_result != 0) {
        BOOL runs_now = app_id != nil && [NSRunningApplication runningApplicationsWithBundleIdentifier:app_id].count > 0;
        if (beebeeb_open_outcome(0, 0, 0, ran_before, runs_now) == 0) {
            NSLog(@"open: no answer within %llds; the file's app was started with it and is still starting: opened",
                  (long long)BeebeebOpenTimeoutSeconds);
            return 0;
        }
        BeebeebFillBridgeError(BeebeebBridgeOpenTimeout, @"Timed out waiting for LaunchServices to open the URL", out_error);
        return -1;
    }
    if (found_error != nil) {
        if (beebeeb_open_outcome(1, handed, 1, ran_before, 0) == 0) {
            NSLog(@"open: the app was handed the file; the system also answered %@ %ld: opened",
                  found_error.domain, (long)found_error.code);
            return 0;
        }
        BeebeebFillError(found_error, out_error);
        return -1;
    }
    return 0;
}

// Task 1885 fix round 1 (I1): the GATED half of "Open in Finder": resolves the Beebeeb domain's root to
// its scoped URL and returns it as a retained handle. Returns: 1 = `*out_handle` set (the caller owns it:
// it MUST pass it to beebeeb_open_scoped_url / beebeeb_reveal_scoped_url / beebeeb_release_url_handle),
// 0 = macOS reported no location (the domain is not added), -1 = error (`out_error` set).
int beebeeb_fp_resolve_location(void **out_handle, BeebeebFpError *out_error) {
    @autoreleasepool {
        *out_handle = NULL;
        int status = -1;
        NSURL *url = BeebeebUserVisibleURL(NSFileProviderRootContainerItemIdentifier, &status, out_error);
        if (status < 0) {
            return -1;
        }
        if (url == nil) {
            return 0;
        }
        *out_handle = BeebeebRetainHandle(url);
        return 1;
    }
}

// The same for one item: the item identifier is the state database's file id (the extension's
// `itemIdentifier` is the daemon's `identifier`). Returns: 1 = handle set, 0 = macOS reported no location,
// -1 = error (`out_error` set; a missing or empty identifier is the bridge's `NoIdentifier`).
int beebeeb_fp_resolve_item(const char *identifier, void **out_handle, BeebeebFpError *out_error) {
    @autoreleasepool {
        *out_handle = NULL;
        NSString *item = (identifier == NULL || identifier[0] == '\0') ? nil : [NSString stringWithUTF8String:identifier];
        if (item == nil) {
            BeebeebFillBridgeError(BeebeebBridgeNoIdentifier, @"no item identifier given", out_error);
            return -1;
        }
        int status = -1;
        NSURL *url = BeebeebUserVisibleURL(item, &status, out_error);
        if (status < 0) {
            return -1;
        }
        if (url == nil) {
            return 0;
        }
        *out_handle = BeebeebRetainHandle(url);
        return 1;
    }
}

// Task 1885 fix round 1 (I1): the UNGATED half. Takes the handle (and always releases it), opens the URL
// in Finder with its scope and returns when LaunchServices has answered. It makes no File Provider call.
// Returns: 1 = opened, -1 = error (`out_error` set).
int beebeeb_open_scoped_url(void *handle, BeebeebFpError *out_error) {
    @autoreleasepool {
        NSURL *url = BeebeebTakeHandle(handle);
        if (url == nil) {
            BeebeebFillBridgeError(BeebeebBridgeInvalidURL, @"no URL handle given", out_error);
            return -1;
        }
        if (BeebeebOpenURLAndWait(url, out_error) != 0) {
            return -1;
        }
        return 1;
    }
}

// "Show in Finder": the ungated half of a reveal. Takes the handle (and always releases it), checks the
// URL is on disk, and selects it in a Finder window. Returns: 1 = handed to Finder, -1 = error.
//
// `activateFileViewerSelectingURLs:` is `void` (NSWorkspace.h:53): Finder's own answer cannot be waited
// for. What this reports is what can be known before it: the item resolved to a URL and the URL exists.
// Whether Finder drew the window is the device check's.
//
// The scope is held after the call, for a bounded time, because the request reaches Finder after this
// function returns and the header does not say when the URL's extension is read.
int beebeeb_reveal_scoped_url(void *handle, BeebeebFpError *out_error) {
    @autoreleasepool {
        NSURL *url = BeebeebTakeHandle(handle);
        if (url == nil) {
            BeebeebFillBridgeError(BeebeebBridgeInvalidURL, @"no URL handle given", out_error);
            return -1;
        }
        BeebeebScope *scope = [[BeebeebScope alloc] initWithURL:url];
        const char *fs_path = url.fileSystemRepresentation;
        struct stat info;
        if (fs_path == NULL || lstat(fs_path, &info) != 0) {
            [scope stop];
            BeebeebFillBridgeError(BeebeebBridgeItemMissing, @"The Finder location of the item is not on disk", out_error);
            return -1;
        }
        [[NSWorkspace sharedWorkspace] activateFileViewerSelectingURLs:@[ url ]];
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, BeebeebOpenTimeoutSeconds * NSEC_PER_SEC),
                       dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
            [scope stop];
        });
        return 1;
    }
}

// dm-72: the app's own read of Finder's copy of an item (a kept file's one copy on a Mac). `handle` is the scoped
// URL beebeeb_fp_resolve_item returned; it stays the caller's (released by beebeeb_release_url_handle as before).
// Writes the URL's path and starts its security scope: the sandbox lets this process read there only between this
// call and beebeeb_fp_end_item_access. It makes no File Provider call and opens nothing. Returns: 1 = path written
// and the scope started (the caller MUST call beebeeb_fp_end_item_access with the same handle), 0 = path written,
// no scope was granted (a read may be refused), -1 = no path (no handle, not a file URL, or the buffer is too small).
int beebeeb_fp_begin_item_access(void *handle, char *path_buffer, unsigned long path_buffer_len) {
    @autoreleasepool {
        if (handle == NULL || path_buffer == NULL || path_buffer_len == 0) {
            return -1;
        }
        NSURL *url = (__bridge NSURL *)handle;
        const char *fs_path = url.isFileURL ? url.fileSystemRepresentation : NULL;
        if (fs_path == NULL || strlen(fs_path) >= path_buffer_len) {
            return -1;
        }
        strlcpy(path_buffer, fs_path, path_buffer_len);
        return [url startAccessingSecurityScopedResource] ? 1 : 0;
    }
}

// dm-72: ends the scope beebeeb_fp_begin_item_access started (only after it returned 1). Does not release the handle.
void beebeeb_fp_end_item_access(void *handle) {
    @autoreleasepool {
        if (handle != NULL) {
            [(__bridge NSURL *)handle stopAccessingSecurityScopedResource];
        }
    }
}

// A handle the caller will not open (an error between the resolve and the open): releases it.
void beebeeb_release_url_handle(void *handle) {
    @autoreleasepool {
        (void)BeebeebTakeHandle(handle);
    }
}

// Task 1885: opens a NON-file URL (System Settings' `x-apple.systempreferences:` pane) through
// NSWorkspace and waits for the answer. A file URL is refused: those open only through the scoped URL
// macOS returned for a Finder item (beebeeb_fp_resolve_location). Returns 0 = opened, -1 = error.
// It touches no File Provider state, so Rust calls it without the bridge gate: the link to System
// Settings must work exactly when the File Provider is stuck.
int beebeeb_open_url(const char *url_string, BeebeebFpError *out_error) {
    @autoreleasepool {
        NSString *text = url_string == NULL ? nil : [NSString stringWithUTF8String:url_string];
        NSURL *url = text.length == 0 ? nil : [NSURL URLWithString:text];
        if (url == nil || url.scheme.length == 0) {
            BeebeebFillBridgeError(BeebeebBridgeInvalidURL, @"The URL does not parse", out_error);
            return -1;
        }
        if (url.isFileURL) {
            BeebeebFillBridgeError(BeebeebBridgeInvalidURL, @"File URLs open only through a Finder item's scoped URL", out_error);
            return -1;
        }
        return BeebeebOpenURLAndWait(url, out_error);
    }
}

// Issue 4 (task 1524): `addDomain` replies success even when the domain already exists and the
// USER has disabled it in System Settings, and a user-disabled domain never launches its
// extension, so a wait for stabilization would burn its full timeout. The install decision
// therefore lives in Rust; this file exposes dumb, synchronous-blocking primitives. Since spec
// 2026-10-06 the decision was the 0.8.x Finder reconciler (`finder_setup/core.rs`, tag
// desktop-v0.8.25; native/beebeeb-drive-rune/docs/macos/file-provider-domain.md).

// Returns: 1 = userEnabled == YES, 0 = userEnabled == NO, 2 = domain not registered,
// -1 = lookup error (out_error set). `userEnabled` has shipped since macOS 11.0, below this
// bridge's 14.0 deployment target (`-mmacosx-version-min=14.0` in the 0.8.x build script).
int beebeeb_fp_domain_user_enabled(BeebeebFpError *out_error) {
    @autoreleasepool {
        NSFileProviderDomain *ours = nil;
        if (BeebeebFindOurDomain(&ours, out_error) != 0) {
            return -1;
        }
        if (ours == nil) {
            return 2;
        }
        return ours.userEnabled ? 1 : 0;
    }
}

// `+[NSFileProviderManager addDomain:completionHandler:]` only; does not wait for
// stabilization. Returns: 0 = success, -1 = error (out_error set).
int beebeeb_fp_add_domain(BeebeebFpError *out_error) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;
        [NSFileProviderManager addDomain:BeebeebDomain() completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        return 0;
    }
}

// Returns: 0 = stable, -1 = timeout or error (out_error set).
int beebeeb_fp_wait_for_domain_ready(double timeout_seconds, BeebeebFpError *out_error) {
    @autoreleasepool {
        return BeebeebWaitForDomainReady(BeebeebDomain(), timeout_seconds, out_error);
    }
}

// Returns: 0 = removed (`kept_state` and `location_buffer` say what was kept), -1 = error
// (`out_error` set). See BeebeebRemoveDomainKeepingUnsynced.
int beebeeb_fp_remove(char *location_buffer,
                      unsigned long location_buffer_len,
                      int *kept_state,
                      BeebeebFpError *out_error) {
    @autoreleasepool {
        return BeebeebRemoveDomainKeepingUnsynced(BeebeebDomain(),
                                                  location_buffer,
                                                  location_buffer_len,
                                                  kept_state,
                                                  out_error);
    }
}

// Task 1697: signal the replica's WORKING SET after the daemon applied an operation batch. Under
// NSFileProviderReplicatedExtension the working set is the ONLY container whose signal the
// system honors; the extension's enumerator then pulls the daemon's change log via ListChanges.
// Returns: 0 = signaled (or domain not registered yet — nothing to signal), -1 = error.
int beebeeb_fp_signal_working_set(BeebeebFpError *out_error) {
    @autoreleasepool {
        NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:BeebeebDomain()];
        if (manager == nil) {
            // Not registered: nothing to signal is not a failure; the engine keeps running.
            return 0;
        }
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;
        [manager signalEnumeratorForContainerItemIdentifier:NSFileProviderWorkingSetContainerItemIdentifier
                                          completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        // Bounded wait: signaling is best-effort UI refresh, never sync state.
        long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
        if (wait_result != 0) {
            BeebeebFillBridgeError(BeebeebBridgeSignalTimeout, @"Timed out signaling the Beebeeb File Provider working set", out_error);
            return -1;
        }
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        return 0;
    }
}

// dm-41: ask the system to forget what it knows below the root and list the whole tree again
// (reimportItemsBelowItemWithIdentifier: root). Called once when the db identity or app generation
// changes. Returns: 0 = asked (or domain not registered: nothing to reimport), -1 = error.
int beebeeb_fp_reimport(BeebeebFpError *out_error) {
    @autoreleasepool {
        NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:BeebeebDomain()];
        if (manager == nil) {
            return 0;
        }
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;
        [manager reimportItemsBelowItemWithIdentifier:NSFileProviderRootContainerItemIdentifier
                                    completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC));
        if (wait_result != 0) {
            BeebeebFillBridgeError(BeebeebBridgeSignalTimeout, @"Timed out asking the Beebeeb File Provider to reimport", out_error);
            return -1;
        }
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        return 0;
    }
}

// dm-65: Settings > Storage > "Downloaded files", and (dm-75) the marks of the rows. The items the system holds on
// disk for this domain (materialised), one per line as "<item identifier>\t<bytes>"; a folder has no size and is left
// out.
// dm-83: the list comes in pages, and every page is read. RC14 read the first page only, and a file Finder had
// downloaded itself was not in it. Rust drives the pages (`macos_downloaded::read_all`), so the loop is tested: open
// the enumerator once, ask one page at a time with the page the last one named, close it at the end.
@interface BeebeebMaterialisedObserver : NSObject <NSFileProviderEnumerationObserver>
@property(nonatomic, strong) NSMutableString *lines;
@property(nonatomic, assign) int count;
@property(nonatomic, strong) NSData *nextPage;
@property(nonatomic, strong) NSError *error;
@property(nonatomic, strong) dispatch_semaphore_t done;
@end
@implementation BeebeebMaterialisedObserver
- (void)didEnumerateItems:(NSArray<id<NSFileProviderItem>> *)updatedItems {
    for (id<NSFileProviderItem> item in updatedItems) {
        NSNumber *size = nil;
        if ([item respondsToSelector:@selector(documentSize)]) {
            size = item.documentSize;
        }
        if (size == nil || item.itemIdentifier.length == 0) {
            continue;
        }
        [self.lines appendFormat:@"%@\t%llu\n", item.itemIdentifier, size.unsignedLongLongValue];
        self.count += 1;
    }
}
- (void)finishEnumeratingUpToPage:(NSFileProviderPage)nextPage {
    // nil: this was the last page.
    self.nextPage = nextPage;
    dispatch_semaphore_signal(self.done);
}
- (void)finishEnumeratingWithError:(NSError *)error {
    self.error = error;
    dispatch_semaphore_signal(self.done);
}
@end

// The enumerator of the materialised items, as a retained handle the caller passes to every page and then to
// beebeeb_fp_materialised_close. NULL: the domain is not registered (no manager), nothing to list.
void *beebeeb_fp_materialised_open(void) {
    @autoreleasepool {
        NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:BeebeebDomain()];
        if (manager == nil) {
            return NULL;
        }
        id<NSFileProviderEnumerator> enumerator = [manager enumeratorForMaterializedItems];
        if (enumerator == nil) {
            return NULL;
        }
        return (void *)CFBridgingRetain(enumerator);
    }
}

// One page. `page`/`page_len`: the page the last one named (length 0: the first page). The page's lines go into
// `buffer` (bounded); the page that follows into `next` (`*next_len` 0: this was the last). Returns the number of
// items on the page (fewer lines than that in `buffer` means it was too small), or -1 (out_error set; also when the
// next page does not fit `next`).
int beebeeb_fp_materialised_page(void *handle,
                                 const unsigned char *page,
                                 unsigned long page_len,
                                 char *buffer,
                                 unsigned long buffer_len,
                                 unsigned char *next,
                                 unsigned long next_cap,
                                 unsigned long *next_len,
                                 BeebeebFpError *out_error) {
    @autoreleasepool {
        if (next_len != NULL) {
            *next_len = 0;
        }
        if (handle == NULL || next_len == NULL) {
            BeebeebFillBridgeError(BeebeebBridgeManagerUnavailable, @"no enumerator handle given", out_error);
            return -1;
        }
        id<NSFileProviderEnumerator> enumerator = (__bridge id<NSFileProviderEnumerator>)handle;
        NSFileProviderPage start = page_len == 0 ? NSFileProviderInitialPageSortedByDate
                                                 : [NSData dataWithBytes:page length:page_len];
        BeebeebMaterialisedObserver *observer = [[BeebeebMaterialisedObserver alloc] init];
        observer.lines = [NSMutableString string];
        observer.done = dispatch_semaphore_create(0);
        [enumerator enumerateItemsForObserver:observer startingAtPage:start];
        long wait_result = dispatch_semaphore_wait(observer.done, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC));
        if (wait_result != 0) {
            BeebeebFillBridgeError(BeebeebBridgeSignalTimeout, @"Timed out listing the downloaded items", out_error);
            return -1;
        }
        if (observer.error != nil) {
            BeebeebFillError(observer.error, out_error);
            return -1;
        }
        NSData *following = observer.nextPage;
        if (following.length > 0) {
            if (next == NULL || following.length > next_cap) {
                BeebeebFillBridgeError(BeebeebBridgeSignalTimeout, @"The next page of the downloaded items is too long", out_error);
                return -1;
            }
            memcpy(next, following.bytes, following.length);
            *next_len = following.length;
        }
        BeebeebCopyString(observer.lines, buffer, buffer_len);
        return observer.count;
    }
}

// Ends the enumeration and lets go of the handle beebeeb_fp_materialised_open gave.
void beebeeb_fp_materialised_close(void *handle) {
    @autoreleasepool {
        if (handle == NULL) {
            return;
        }
        id<NSFileProviderEnumerator> enumerator = (id<NSFileProviderEnumerator>)CFBridgingRelease(handle);
        if ([enumerator respondsToSelector:@selector(invalidate)]) {
            [enumerator invalidate];
        }
    }
}

// dm-65: make ONE item cloud-only again (evictItemWithIdentifier:), the call Finder's "Remove Download" makes.
// Returns 0 = evicted (or domain not registered), -1 (out_error set).
int beebeeb_fp_evict_item(const char *identifier, BeebeebFpError *out_error) {
    @autoreleasepool {
        if (identifier == NULL || identifier[0] == 0) {
            BeebeebFillBridgeError(BeebeebBridgeNoIdentifier, @"no item identifier given", out_error);
            return -1;
        }
        NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:BeebeebDomain()];
        if (manager == nil) {
            return 0;
        }
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;
        [manager evictItemWithIdentifier:[NSString stringWithUTF8String:identifier]
                       completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC));
        if (wait_result != 0) {
            BeebeebFillBridgeError(BeebeebBridgeSignalTimeout, @"Timed out making an item cloud-only", out_error);
            return -1;
        }
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        return 0;
    }
}

// dm-71: keep ONE item on this Mac through the system: ask it to download the item now
// (requestDownloadForItemWithIdentifier:, the whole file). The extension's fetchContents is then the one download;
// the item's content policy (downloadEagerlyAndKeepDownloaded while it is kept) keeps the copy.
// dm-75: this is the ONE request every open makes too (the viewer, Open with, the list's Open; Rust's
// `macos_fp::materialise`). The request is a scheduling hint ("at the earliest convenient time",
// NSFileProviderManager.h, macOS 13.0+), so the Rust side also reads the file and waits until it is no longer
// dataless. A domain with no manager is an error now, never "asked": nothing was asked, and a caller that waits for
// the file must hear that.
// Returns 0 = the system took the request, -1 (out_error set).
int beebeeb_fp_request_download(const char *identifier, BeebeebFpError *out_error) {
    @autoreleasepool {
        if (identifier == NULL || identifier[0] == 0) {
            BeebeebFillBridgeError(BeebeebBridgeNoIdentifier, @"no item identifier given", out_error);
            return -1;
        }
        NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:BeebeebDomain()];
        if (manager == nil) {
            BeebeebFillBridgeError(BeebeebBridgeManagerUnavailable,
                                   @"File Provider manager is unavailable for the Beebeeb domain", out_error);
            return -1;
        }
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;
        [manager requestDownloadForItemWithIdentifier:[NSString stringWithUTF8String:identifier]
                                       requestedRange:NSMakeRange(NSNotFound, 0)
                                    completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC));
        if (wait_result != 0) {
            BeebeebFillBridgeError(BeebeebBridgeSignalTimeout, @"Timed out asking for an item's download", out_error);
            return -1;
        }
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        return 0;
    }
}

// Task 1698 part 3: every registered identifier of THIS provider, newline-separated.
// Returns the count, or -1 (out_error set).
int beebeeb_fp_list_domains(char *ids_buffer, unsigned long ids_buffer_len, BeebeebFpError *out_error) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSArray<NSFileProviderDomain *> *found_domains = nil;
        __block NSError *found_error = nil;
        [NSFileProviderManager getDomainsWithCompletionHandler:^(NSArray<NSFileProviderDomain *> *domains, NSError *error) {
            found_domains = domains;
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        NSMutableString *joined = [NSMutableString string];
        for (NSFileProviderDomain *domain in found_domains) {
            if (joined.length > 0) {
                [joined appendString:@"\n"];
            }
            [joined appendString:domain.identifier ?: @""];
        }
        BeebeebCopyString(joined, ids_buffer, ids_buffer_len);
        return (int)found_domains.count;
    }
}

// Task 1698 part 3: remove ONE domain by identifier. Task 1882: it keeps un-synced files like
// every removal. Returns 0 = removed (or already gone; `kept_state` and `location_buffer` say what
// was kept), -1 (out_error set).
int beebeeb_fp_remove_domain_by_id(const char *identifier,
                                   char *location_buffer,
                                   unsigned long location_buffer_len,
                                   int *kept_state,
                                   BeebeebFpError *out_error) {
    @autoreleasepool {
        if (identifier == NULL) {
            BeebeebFillBridgeError(BeebeebBridgeNoIdentifier, @"no domain identifier given", out_error);
            return -1;
        }
        NSString *domain_id = [NSString stringWithUTF8String:identifier];
        NSFileProviderDomain *domain = [[NSFileProviderDomain alloc] initWithIdentifier:domain_id displayName:domain_id];
        return BeebeebRemoveDomainKeepingUnsynced(domain,
                                                  location_buffer,
                                                  location_buffer_len,
                                                  kept_state,
                                                  out_error);
    }
}

// ── Test hooks (spec §13.1 "Bridge") ─────────────────────────────────────────
// Called only from Rust tests in macos_file_provider.rs. They touch no system service.
//
// Build a real NSError (with an underlying error when `underlying_domain` is non-NULL) and
// run it through the SAME BeebeebFillError every call above uses.
void beebeeb_fp_test_fill_error(const char *domain, int64_t code, const char *message,
                                const char *underlying_domain, int64_t underlying_code,
                                BeebeebFpError *out_error) {
    @autoreleasepool {
        NSMutableDictionary *info = [NSMutableDictionary dictionary];
        info[NSLocalizedDescriptionKey] = [NSString stringWithUTF8String:message ?: ""];
        if (underlying_domain != NULL) {
            info[NSUnderlyingErrorKey] = [NSError errorWithDomain:[NSString stringWithUTF8String:underlying_domain]
                                                             code:(NSInteger)underlying_code
                                                         userInfo:nil];
        }
        NSError *error = [NSError errorWithDomain:[NSString stringWithUTF8String:domain ?: ""]
                                             code:(NSInteger)code
                                         userInfo:info];
        BeebeebFillError(error, out_error);
    }
}

void beebeeb_fp_test_fill_bridge_error(int64_t code, const char *message, BeebeebFpError *out_error) {
    @autoreleasepool {
        BeebeebFillBridgeError(code, [NSString stringWithUTF8String:message ?: ""], out_error);
    }
}

unsigned long beebeeb_fp_test_error_size(void) {
    return sizeof(BeebeebFpError);
}

unsigned long beebeeb_fp_test_error_align(void) {
    return _Alignof(BeebeebFpError);
}

// `out` holds 6 values: the offsets of code, underlying_code, has_underlying, domain,
// underlying_domain, message, in declaration order.
void beebeeb_fp_test_error_offsets(unsigned long *out) {
    if (out == NULL) {
        return;
    }
    out[0] = offsetof(BeebeebFpError, code);
    out[1] = offsetof(BeebeebFpError, underlying_code);
    out[2] = offsetof(BeebeebFpError, has_underlying);
    out[3] = offsetof(BeebeebFpError, domain);
    out[4] = offsetof(BeebeebFpError, underlying_domain);
    out[5] = offsetof(BeebeebFpError, message);
}

// Task 1885 fix round 1: a handle for an arbitrary URL string (any scheme, including a file URL), made by the
// same call the resolve uses, so the tests can drive the ungated open and reveal without a File Provider. NULL
// when the string does not parse. And how many handles are in flight, so a test can see none leaks.
void *beebeeb_test_url_handle(const char *url_string) {
    @autoreleasepool {
        NSString *text = url_string == NULL ? nil : [NSString stringWithUTF8String:url_string];
        NSURL *url = text.length == 0 ? nil : [NSURL URLWithString:text];
        return url == nil ? NULL : BeebeebRetainHandle(url);
    }
}

int64_t beebeeb_test_live_handles(void) {
    return __atomic_load_n(&BeebeebLiveHandles, __ATOMIC_SEQ_CST);
}

// `out` holds 8 values: ManagerUnavailable, StabilizationTimeout, ResolveUrlTimeout,
// SignalTimeout, NoIdentifier, OpenTimeout, InvalidURL, ItemMissing.
void beebeeb_fp_test_bridge_codes(int64_t *out) {
    if (out == NULL) {
        return;
    }
    out[0] = BeebeebBridgeManagerUnavailable;
    out[1] = BeebeebBridgeStabilizationTimeout;
    out[2] = BeebeebBridgeResolveUrlTimeout;
    out[3] = BeebeebBridgeSignalTimeout;
    out[4] = BeebeebBridgeNoIdentifier;
    out[5] = BeebeebBridgeOpenTimeout;
    out[6] = BeebeebBridgeInvalidURL;
    out[7] = BeebeebBridgeItemMissing;
}
