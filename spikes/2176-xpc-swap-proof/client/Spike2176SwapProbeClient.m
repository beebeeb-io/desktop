// spike(2176): the app-side trigger of the XPC swap device proof. THROWAWAY, never merged.
//
// Compiled and linked ONLY with `cargo build --features spike-2176` (src-tauri/build.rs, the same
// clang + ar + static-link pattern as macos/FileProviderBridge.m). A normal build never compiles it.
// No Rust source references it: the linker is told to keep it (`-u _beebeeb_spike_2176_anchor`), and
// a constructor starts it when the process loads, so `src-tauri/src` stays byte-for-byte unchanged.
//
// It acts only when the app is launched with BEEBEEB_SPIKE_2176 set, e.g.
//   open -n --env BEEBEEB_SPIKE_2176=p1 --env BEEBEEB_SPIKE_2176_RUN=p1-1 /Applications/Beebeeb.app
// Commands:
//   ping                          ask the service who it is
//   p1                            ping, then writeProbe(<home>/beebeeb-spike-2176/p1-probe.txt)
//   swap:<staged>                 swap(<staged>, <this app's bundle path>); the app keeps running
//   swap-relaunch:<staged>        swap, and on ok open a new instance of the bundle, then quit
//   swap-after-quit:<staged>      the service arms, this app quits, the service swaps and relaunches
//   swap-dest:<staged>|<dest>     swap with an explicit destination (negative test only)
//   cleanup                       removeStaleUpdates(<this app's bundle path>)
//   watch                         poll <container>/Library/Application Support/BeebeebSpike2176/command
//                                 once a second for "<run id> <command>" lines (P4, negative cases)
// Results: JSON at <container>/Library/Application Support/BeebeebSpike2176/results/<run>.json.
// Results hold codes, pids and versions, never a path; nothing is written to lifecycle.log.

#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#include <pwd.h>
#include <stdlib.h>
#include <unistd.h>

static NSString *const kServiceName = @"io.beebeeb.app.SwapProbe";

// The service must be Beebeeb's own, Developer ID signed by team R8352WDJJR (TN3127 form; the same
// requirement the service applies to this app, with the service's identifier).
static NSString *const kServiceRequirement =
    @"anchor apple generic and identifier \"io.beebeeb.app.SwapProbe\""
    @" and certificate 1[field.1.2.840.113635.100.6.2.6] exists"
    @" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
    @" and certificate leaf[subject.OU] = \"R8352WDJJR\"";

typedef void (^BBSpikeReply)(NSString *code, NSString *stage, int32_t err);

// Must match BeebeebSwapProbeProtocol in BeebeebSwapProbe/SwapProbeService.swift selector for selector.
@protocol BeebeebSwapProbeProtocol
- (void)pingWithReply:(void (^)(NSString *info))reply;
- (void)writeProbe:(NSString *)path withReply:(void (^)(NSString *code, NSString *stage, int32_t err))reply;
- (void)swap:(NSString *)stagedPath
    destinationPath:(NSString *)destinationPath
          withReply:(void (^)(NSString *code, NSString *stage, int32_t err))reply;
- (void)swapAfterClientExit:(NSString *)stagedPath
            destinationPath:(NSString *)destinationPath
                      runID:(NSString *)runID
                  withReply:(void (^)(NSString *code, NSString *stage, int32_t err))reply;
- (void)removeStaleUpdates:(NSString *)destinationPath
                 withReply:(void (^)(NSString *code, NSString *stage, int32_t err))reply;
@end

// Referenced by the linker flag `-u _beebeeb_spike_2176_anchor` (build.rs) so this object, and its
// constructor below, is linked into the app even though no Rust code calls into it.
__attribute__((used, visibility("default"))) void beebeeb_spike_2176_anchor(void) {}

static NSString *BBSpikeDir(void) {
    // NSHomeDirectory() is the container's Data directory in the sandboxed app.
    return [NSHomeDirectory() stringByAppendingPathComponent:@"Library/Application Support/BeebeebSpike2176"];
}

static NSString *BBRealHome(void) {
    struct passwd *pw = getpwuid(getuid());
    return (pw != NULL && pw->pw_dir != NULL) ? @(pw->pw_dir) : nil;
}

static BOOL BBIsRunID(NSString *run) {
    if (run.length == 0 || run.length > 40) return NO;
    NSCharacterSet *allowed =
        [NSCharacterSet characterSetWithCharactersInString:@"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-"];
    return [run rangeOfCharacterFromSet:allowed.invertedSet].location == NSNotFound;
}

static void BBWriteResult(NSString *run, NSDictionary *fields) {
    NSMutableDictionary *d = [fields mutableCopy];
    d[@"run"] = run;
    d[@"client_pid"] = @(getpid());
    d[@"client_version"] = [NSBundle.mainBundle objectForInfoDictionaryKey:@"CFBundleShortVersionString"] ?: @"unknown";
    d[@"client_home_is_container"] = @([NSHomeDirectory() containsString:@"/Library/Containers/"]);
    d[@"t"] = [[NSISO8601DateFormatter new] stringFromDate:[NSDate date]];
    NSString *dir = [BBSpikeDir() stringByAppendingPathComponent:@"results"];
    [NSFileManager.defaultManager createDirectoryAtPath:dir
                            withIntermediateDirectories:YES
                                             attributes:@{NSFilePosixPermissions: @0700}
                                                  error:nil];
    NSData *json = [NSJSONSerialization dataWithJSONObject:d
                                                   options:NSJSONWritingPrettyPrinted | NSJSONWritingSortedKeys
                                                     error:nil];
    [json writeToFile:[dir stringByAppendingPathComponent:[run stringByAppendingString:@".json"]] atomically:YES];
}

static NSXPCConnection *BBConnect(void) {
    NSXPCConnection *c = [[NSXPCConnection alloc] initWithServiceName:kServiceName];
    c.remoteObjectInterface = [NSXPCInterface interfaceWithProtocol:@protocol(BeebeebSwapProbeProtocol)];
    if (@available(macOS 13.0, *)) {
        [c setCodeSigningRequirement:kServiceRequirement];
    }
    [c resume];
    return c;
}

// Calls one service method and waits for (code, stage, err), an XPC error, or the timeout.
static NSDictionary *BBCall(NSXPCConnection *c, NSTimeInterval timeout,
                            void (^invoke)(id<BeebeebSwapProbeProtocol> proxy, BBSpikeReply reply)) {
    __block NSDictionary *result = nil;
    NSObject *lock = [NSObject new];
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    id<BeebeebSwapProbeProtocol> proxy = [c remoteObjectProxyWithErrorHandler:^(NSError *error) {
        @synchronized(lock) {
            if (result == nil) {
                result = @{@"code": @"xpc_error", @"stage": @"connection", @"err": @(error.code), @"xpc_domain": error.domain};
            }
        }
        dispatch_semaphore_signal(done);
    }];
    invoke(proxy, ^(NSString *code, NSString *stage, int32_t err) {
        @synchronized(lock) {
            if (result == nil) result = @{@"code": code ?: @"", @"stage": stage ?: @"", @"err": @(err)};
        }
        dispatch_semaphore_signal(done);
    });
    if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(timeout * NSEC_PER_SEC))) != 0) {
        @synchronized(lock) {
            if (result == nil) result = @{@"code": @"timeout", @"stage": @"connection", @"err": @0};
        }
    }
    @synchronized(lock) {
        return result;
    }
}

static NSDictionary *BBPing(NSXPCConnection *c) {
    __block NSString *info = nil;
    NSDictionary *r = BBCall(c, 20, ^(id<BeebeebSwapProbeProtocol> proxy, BBSpikeReply reply) {
        [proxy pingWithReply:^(NSString *i) {
            info = i;
            reply(@"ok", @"ping", 0);
        }];
    });
    NSMutableDictionary *out = [r mutableCopy];
    if (info) out[@"service_info"] = info;
    return out;
}

// Quit this app the way a person would, with a watchdog in case the quit is swallowed.
static void BBQuitSelf(NSString *run) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC), dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        BBWriteResult([run stringByAppendingString:@"-exit"], @{@"code": @"watchdog_exit", @"stage": @"quit", @"err": @0});
        exit(0);
    });
    dispatch_async(dispatch_get_main_queue(), ^{
        [NSApp terminate:nil];
    });
}

static void BBExecute(NSString *command, NSString *run) {
    NSString *verb = command;
    NSString *arg = @"";
    NSRange colon = [command rangeOfString:@":"];
    if (colon.location != NSNotFound) {
        verb = [command substringToIndex:colon.location];
        arg = [command substringFromIndex:colon.location + 1];
    }
    NSString *bundlePath = NSBundle.mainBundle.bundlePath;
    NSXPCConnection *c = BBConnect();
    NSMutableDictionary *result = [NSMutableDictionary dictionaryWithObject:verb forKey:@"mode"];
    BOOL quitAfter = NO;

    if ([verb isEqualToString:@"ping"]) {
        [result addEntriesFromDictionary:BBPing(c)];
    } else if ([verb isEqualToString:@"p1"]) {
        NSDictionary *ping = BBPing(c);
        result[@"service_info"] = ping[@"service_info"] ?: @"";
        result[@"ping_code"] = ping[@"code"];
        NSString *home = BBRealHome();
        NSString *probe = [home stringByAppendingPathComponent:@"beebeeb-spike-2176/p1-probe.txt"];
        [result addEntriesFromDictionary:BBCall(c, 20, ^(id<BeebeebSwapProbeProtocol> proxy, BBSpikeReply reply) {
                    [proxy writeProbe:probe withReply:reply];
                })];
    } else if ([verb isEqualToString:@"swap"] || [verb isEqualToString:@"swap-relaunch"] ||
               [verb isEqualToString:@"swap-dest"]) {
        NSString *staged = arg;
        NSString *destination = bundlePath;
        if ([verb isEqualToString:@"swap-dest"]) {
            NSArray<NSString *> *parts = [arg componentsSeparatedByString:@"|"];
            staged = parts.firstObject;
            destination = parts.count > 1 ? parts[1] : @"";
        }
        NSDictionary *ping = BBPing(c);
        result[@"service_info"] = ping[@"service_info"] ?: @"";
        [result addEntriesFromDictionary:BBCall(c, 300, ^(id<BeebeebSwapProbeProtocol> proxy, BBSpikeReply reply) {
                    [proxy swap:staged destinationPath:destination withReply:reply];
                })];
        if ([verb isEqualToString:@"swap-relaunch"] && [result[@"code"] isEqualToString:@"ok"]) {
            // Open a NEW instance of the (now swapped) bundle, then quit this one: the order the Tauri
            // updater uses (start the new process, then exit), done through LaunchServices so the new
            // process is a proper launch and not a child that inherits this one's sandbox.
            dispatch_semaphore_t opened = dispatch_semaphore_create(0);
            __block NSString *relaunch = @"timeout";
            __block pid_t newPid = 0;
            NSWorkspaceOpenConfiguration *cfg = [NSWorkspaceOpenConfiguration configuration];
            cfg.createsNewApplicationInstance = YES;
            cfg.activates = YES;
            [NSWorkspace.sharedWorkspace openApplicationAtURL:NSBundle.mainBundle.bundleURL
                                                configuration:cfg
                                            completionHandler:^(NSRunningApplication *app, NSError *error) {
                                                if (error) {
                                                    relaunch = [NSString stringWithFormat:@"failed %@ %ld", error.domain, (long)error.code];
                                                } else {
                                                    relaunch = @"ok";
                                                    newPid = app.processIdentifier;
                                                }
                                                dispatch_semaphore_signal(opened);
                                            }];
            dispatch_semaphore_wait(opened, dispatch_time(DISPATCH_TIME_NOW, 30 * NSEC_PER_SEC));
            result[@"relaunch"] = relaunch;
            result[@"relaunch_pid"] = @(newPid);
            quitAfter = YES;
        }
    } else if ([verb isEqualToString:@"swap-after-quit"]) {
        NSDictionary *ping = BBPing(c);
        result[@"service_info"] = ping[@"service_info"] ?: @"";
        [result addEntriesFromDictionary:BBCall(c, 30, ^(id<BeebeebSwapProbeProtocol> proxy, BBSpikeReply reply) {
                    [proxy swapAfterClientExit:arg destinationPath:bundlePath runID:run withReply:reply];
                })];
        quitAfter = [result[@"code"] isEqualToString:@"ok"];
    } else if ([verb isEqualToString:@"cleanup"]) {
        [result addEntriesFromDictionary:BBCall(c, 60, ^(id<BeebeebSwapProbeProtocol> proxy, BBSpikeReply reply) {
                    [proxy removeStaleUpdates:bundlePath withReply:reply];
                })];
    } else {
        result[@"code"] = @"unknown_command";
    }
    [c invalidate];
    if (quitAfter) result[@"quit"] = @"terminate_requested";
    BBWriteResult(run, result);
    if (quitAfter) BBQuitSelf(run);
}

static void BBWatch(void) {
    static dispatch_source_t timer;
    dispatch_queue_t q = dispatch_queue_create("io.beebeeb.app.spike2176.watch", DISPATCH_QUEUE_SERIAL);
    timer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, q);
    dispatch_source_set_timer(timer, dispatch_time(DISPATCH_TIME_NOW, NSEC_PER_SEC), NSEC_PER_SEC, NSEC_PER_SEC / 10);
    dispatch_source_set_event_handler(timer, ^{
        NSString *path = [BBSpikeDir() stringByAppendingPathComponent:@"command"];
        NSString *taken = [BBSpikeDir() stringByAppendingPathComponent:@"command.taken"];
        if (rename(path.fileSystemRepresentation, taken.fileSystemRepresentation) != 0) return;
        NSString *text = [NSString stringWithContentsOfFile:taken encoding:NSUTF8StringEncoding error:nil];
        unlink(taken.fileSystemRepresentation);
        NSString *line = [text stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
        NSRange space = [line rangeOfString:@" "];
        if (space.location == NSNotFound) return;
        NSString *run = [line substringToIndex:space.location];
        NSString *command = [line substringFromIndex:space.location + 1];
        if (!BBIsRunID(run)) return;
        BBExecute(command, run);
    });
    dispatch_resume(timer);
}

__attribute__((constructor)) static void BBSpike2176Start(void) {
    const char *raw = getenv("BEEBEEB_SPIKE_2176");
    if (raw == NULL || raw[0] == '\0') return;
    NSString *command = @(raw);
    const char *rawRun = getenv("BEEBEEB_SPIKE_2176_RUN");
    NSString *run = rawRun ? @(rawRun) : @"norun";
    if (!BBIsRunID(run)) run = @"badrun";
    // Not inherited by anything this app starts (a helper, a relaunch).
    unsetenv("BEEBEEB_SPIKE_2176");
    unsetenv("BEEBEEB_SPIKE_2176_RUN");
    if ([command isEqualToString:@"watch"]) {
        BBWatch();
        return;
    }
    // Let the app finish starting, then act once.
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC), dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        BBExecute(command, run);
    });
}
