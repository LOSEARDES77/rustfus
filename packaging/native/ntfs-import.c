/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Import a private staged tree into a regular NTFS image, without FUSE. */
#include "config.h"
#include <sys/stat.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include "volume.h"
#include "dir.h"
#include "attrib.h"
#include "unistr.h"

static int transfer(ntfs_volume *volume, int directory, const char *path, int verify, unsigned depth)
{
    if (depth > 128) { errno = ELOOP; return -1; }
    DIR *entries = fdopendir(dup(directory));
    if (!entries) return -1;
    int result = 0;
    struct dirent *entry;
    while (1) {
        errno = 0;
        entry = readdir(entries);
        if (!entry) { if (errno) result = -1; break; }
        if (!strcmp(entry->d_name, ".") || !strcmp(entry->d_name, "..")) continue;
        if (strpbrk(entry->d_name, "\\:") || entry->d_name[0] == '$') { errno = EINVAL; result = -1; break; }
        char *child = NULL;
        if (asprintf(&child, "%s/%s", path, entry->d_name) < 0) { result = -1; break; }
        int source = openat(directory, entry->d_name, O_RDONLY | O_NOFOLLOW | O_NONBLOCK);
        struct stat status;
        if (source < 0 || fstat(source, &status) ||
            (!S_ISREG(status.st_mode) && !S_ISDIR(status.st_mode)) ||
            (S_ISREG(status.st_mode) && status.st_nlink != 1)) {
            if (source >= 0) close(source);
            free(child); result = -1; break;
        }
        ntfs_inode *inode = NULL;
        if (verify) {
            inode = ntfs_pathname_to_inode(volume, NULL, child);
        } else {
            ntfs_inode *parent = ntfs_pathname_to_inode(volume, NULL, *path ? path : "/");
            ntfschar *name = NULL;
            int length = ntfs_mbstoucs(entry->d_name, &name);
            if (parent && length > 0 && length <= 255)
                inode = ntfs_create(parent, const_cpu_to_le32(0), name, length,
                                    S_ISDIR(status.st_mode) ? S_IFDIR : S_IFREG);
            free(name);
            if (parent && ntfs_inode_close(parent)) result = -1;
        }
        if (!inode || result) {
            fprintf(stderr, "Opening/creating NTFS inode %s: %s\n", child, strerror(errno));
            if (inode) ntfs_inode_close(inode);
            close(source); free(child); result = -1; break;
        }
        if (S_ISDIR(status.st_mode)) {
            if (!(inode->mrec->flags & MFT_RECORD_IS_DIRECTORY)) result = -1;
            if (inode && ntfs_inode_close(inode)) result = -1;
            inode = NULL;
            if (!result && transfer(volume, source, child, verify, depth + 1)) result = -1;
        } else {
            ntfs_attr *attribute = ntfs_attr_open(inode, AT_DATA, AT_UNNAMED, 0);
            if (!attribute) { perror("Opening NTFS data stream"); result = -1; }
            else {
                if ((!verify && ntfs_attr_truncate(attribute, status.st_size)) ||
                    (verify && attribute->data_size != status.st_size)) { perror("Setting NTFS data size"); result = -1; }
                char *buffer = malloc(2097152);
                char *actual = buffer ? buffer + 1048576 : NULL;
                if (!buffer) result = -1;
                s64 position = 0;
                while (!result) {
                    ssize_t count = read(source, buffer, 1048576);
                    if (count < 0) { if (errno == EINTR) continue; result = -1; break; }
                    if (!count) break;
                    s64 done = 0;
                    while (done < count) {
                        s64 size = verify ? ntfs_attr_pread(attribute, position + done, count - done, actual + done)
                                          : ntfs_attr_pwrite(attribute, position + done, count - done, buffer + done);
                        if (size <= 0) { result = -1; break; }
                        done += size;
                    }
                    if (verify && !result && memcmp(buffer, actual, count)) { errno = EIO; result = -1; }
                    position += count;
                }
                if (position != status.st_size) result = -1;
                free(buffer);
                ntfs_attr_close(attribute);
            }
        }
        if (ntfs_inode_close(inode)) result = -1;
        close(source);
        if (result) fprintf(stderr, "NTFS %s failed: %s (%s)\n", verify ? "verification" : "import", child, strerror(errno));
        free(child);
        if (result) break;
    }
    closedir(entries);
    return result;
}

int main(int argc, char **argv)
{
    if (argc != 4 || (strcmp(argv[1], "import") && strcmp(argv[1], "verify"))) {
        fprintf(stderr, "Usage: ntfs-import import|verify PRIVATE_IMAGE STAGED_DIRECTORY\n");
        return 2;
    }
    struct stat status;
    if (lstat(argv[2], &status) || !S_ISREG(status.st_mode) || status.st_nlink != 1) {
        fprintf(stderr, "NTFS helper accepts regular private image files only\n");
        return 2;
    }
    int directory = open(argv[3], O_RDONLY | O_DIRECTORY | O_NOFOLLOW);
    if (directory < 0 || ntfs_set_char_encoding("UTF-8")) return 2;
    int verify = !strcmp(argv[1], "verify");
    ntfs_volume *volume = ntfs_mount(argv[2], verify ? NTFS_MNT_RDONLY : NTFS_MNT_EXCLUSIVE);
    if (!volume) { perror("Opening private NTFS image"); close(directory); return 1; }
    int result = transfer(volume, directory, "", verify, 0);
    if (ntfs_umount(volume, FALSE)) { perror("Flushing private NTFS image"); result = -1; }
    close(directory);
    return result ? 1 : 0;
}
