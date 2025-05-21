import frida
import sys

# Target process (e.g., explorer.exe, obs64.exe, etc.)
target_process = "iWinSupport.exe"  # You can change this

# Frida JavaScript code to hook screen capture functions
js_code = """
Interceptor.attach(Module.getExportByName("gdi32.dll", "BitBlt"), {
    onEnter: function (args) {
        console.log("[BitBlt] Screen capture function called.");
    }
});

Interceptor.attach(Module.getExportByName("user32.dll", "PrintWindow"), {
    onEnter: function (args) {
        console.log("[PrintWindow] Screen capture function called.");
    }
});

Interceptor.attach(Module.getExportByName("user32.dll", "GetDC"), {
    onEnter: function (args) {
        console.log("[GetDC] Possibly capturing screen.");
    }
});
"""

def on_message(message, data):
    if message['type'] == 'send':
        print("[*] {}".format(message['payload']))
    elif message['type'] == 'error':
        print("[!] Error:", message['stack'])

try:
    session = frida.attach(target_process)
    script = session.create_script(js_code)
    script.on('message', on_message)
    script.load()
    print(f"✅ Monitoring '{target_process}' for screen capture API usage. Press Ctrl+C to stop.")
    sys.stdin.read()
except frida.ProcessNotFoundError:
    print(f"❌ Process '{target_process}' not found. Make sure it's running.")
except Exception as e:
    print("❌ Error:", e)
