/* Extract fields using the SDK's kinfo_proc layout for the target architecture. */
#include <stdint.h>
#include <stdlib.h>
#include <sys/proc.h>
#include <sys/sysctl.h>

struct process_info {
    uint64_t started_at;
    int64_t started_at_micros;
    pid_t pid;
    pid_t parent;
    uid_t uid;
    dev_t tty;
    uint32_t zombie;
};

static struct process_info process_info(const struct kinfo_proc *record) {
    return (struct process_info) {
        .started_at = record->kp_proc.p_starttime.tv_sec,
        .started_at_micros = record->kp_proc.p_starttime.tv_usec,
        .pid = record->kp_proc.p_pid,
        .parent = record->kp_eproc.e_ppid,
        .uid = record->kp_eproc.e_ucred.cr_ref == 0
            ? UINT32_MAX : record->kp_eproc.e_ucred.cr_uid,
        .tty = record->kp_eproc.e_tdev,
        .zombie = record->kp_proc.p_stat == SZOMB,
    };
}

int gwt_switch_process_info(pid_t pid, struct process_info *out) {
    int mib[] = {CTL_KERN, KERN_PROC, KERN_PROC_PID, pid};
    struct kinfo_proc record;
    size_t size = sizeof(record);
    if (sysctl(mib, 4, &record, &size, NULL, 0) != 0 || size != sizeof(record)) {
        return 0;
    }
    *out = process_info(&record);
    return 1;
}

int gwt_switch_process_snapshot(void *context,
                                void (*emit)(void *, const struct process_info *)) {
    int mib[] = {CTL_KERN, KERN_PROC, KERN_PROC_ALL, 0};
    size_t size = 0;
    if (sysctl(mib, 4, NULL, &size, NULL, 0) != 0 || size == 0) {
        return 0;
    }
    struct kinfo_proc *records = malloc(size);
    if (records == NULL) {
        return 0;
    }
    int complete = sysctl(mib, 4, records, &size, NULL, 0) == 0
        && size % sizeof(*records) == 0;
    if (complete) {
        for (size_t index = 0; index < size / sizeof(*records); ++index) {
            struct process_info process = process_info(&records[index]);
            emit(context, &process);
        }
    }
    free(records);
    return complete;
}
