#include <windows.h>
#include <dbgeng.h>
#include <cstdio>
#include <cstdlib>
int main(int argc, char** argv) {
    if (argc != 3) {
        fprintf(stderr, "Usage: capture-windows-stacks PID LOG_PATH\n");
        return 2;
    }
    char* end = nullptr;
    const unsigned long process_id = strtoul(argv[1], &end, 10);
    if (!process_id || *end) return 2;
    IDebugClient* client = nullptr;
    IDebugControl* control = nullptr;
    HRESULT hr = DebugCreate(__uuidof(IDebugClient), (void**)&client);
    if (FAILED(hr)) { printf("DebugCreate %08lx\n", hr); return 3; }
    hr = client->QueryInterface(__uuidof(IDebugControl), (void**)&control);
    if (FAILED(hr)) { client->Release(); return 4; }
    hr = control->OpenLogFile(argv[2], FALSE);
    if (FAILED(hr)) { control->Release(); client->Release(); return 4; }
    // Read-only attachment without suspending the target. Frames may race execution.
    hr = client->AttachProcess(0, process_id,
        DEBUG_ATTACH_NONINVASIVE | DEBUG_ATTACH_NONINVASIVE_NO_SUSPEND);
    printf("AttachProcess %08lx\n", hr);
    if (SUCCEEDED(hr)) hr = control->WaitForEvent(0, 10000);
    printf("WaitForEvent %08lx\n", hr);
    if (SUCCEEDED(hr)) {
        hr = control->Execute(DEBUG_OUTCTL_ALL_CLIENTS, ".time; lm m gwt; ~* k 16", DEBUG_EXECUTE_DEFAULT);
    }
    client->DetachProcesses();
    control->CloseLogFile();
    control->Release(); client->Release();
    return FAILED(hr) ? 5 : 0;
}
