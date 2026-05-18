#import <Foundation/Foundation.h>
#import <FileProvider/FileProvider.h>
#import <dispatch/dispatch.h>
#include <string.h>

static NSString *BeebeebDomainIdentifier = @"io.beebeeb.desktop.domain";
static NSString *BeebeebDomainDisplayName = @"Beebeeb";

static void BeebeebCopyMessage(NSString *message, char *buffer, unsigned long buffer_len) {
    if (buffer == NULL || buffer_len == 0) {
        return;
    }
    const char *utf8 = [message UTF8String];
    strlcpy(buffer, utf8 ?: "unknown File Provider error", buffer_len);
}

static void BeebeebCopyError(NSError *error, char *buffer, unsigned long buffer_len) {
    NSString *message = [NSString stringWithFormat:@"%@ (%@ %ld)",
                                                   error.localizedDescription ?: @"unknown File Provider error",
                                                   error.domain ?: @"unknown-domain",
                                                   (long)error.code];
    BeebeebCopyMessage(message, buffer, buffer_len);
}

static NSFileProviderDomain *BeebeebDomain(void) {
    return [[NSFileProviderDomain alloc] initWithIdentifier:BeebeebDomainIdentifier
                                               displayName:BeebeebDomainDisplayName];
}

int beebeeb_fp_status(char *error_buffer, unsigned long error_buffer_len) {
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
            BeebeebCopyError(found_error, error_buffer, error_buffer_len);
            return -1;
        }
        for (NSFileProviderDomain *domain in found_domains) {
            if ([domain.identifier isEqualToString:BeebeebDomainIdentifier]) {
                return 1;
            }
        }
        return 0;
    }
}

int beebeeb_fp_install(char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;

        [NSFileProviderManager addDomain:BeebeebDomain() completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);

        if (found_error != nil) {
            BeebeebCopyError(found_error, error_buffer, error_buffer_len);
            return -1;
        }
        return 0;
    }
}

int beebeeb_fp_remove(char *error_buffer, unsigned long error_buffer_len) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;

        [NSFileProviderManager removeDomain:BeebeebDomain() completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);

        if (found_error != nil) {
            BeebeebCopyError(found_error, error_buffer, error_buffer_len);
            return -1;
        }
        return 0;
    }
}
