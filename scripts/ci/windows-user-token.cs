// The hosted Windows runner is elevated. WebView2 150 and later do not
// expose their automation endpoint at high integrity. Start the test process
// with the same account, restricted privileges and medium integrity.
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace MannieCi
{
    public static class StandardUser
    {
        [StructLayout(LayoutKind.Sequential)]
        private struct MandatoryLabel
        {
            public IntPtr Sid;
            public uint Attributes;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct StartupInfo
        {
            public int Size;
            public string Reserved;
            public string Desktop;
            public string Title;
            public uint X, Y, Width, Height, BufferWidth, BufferHeight;
            public uint FillAttribute, Flags;
            public ushort ShowWindow, ReservedSize;
            public IntPtr ReservedData, StandardInput, StandardOutput, StandardError;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct ProcessInformation
        {
            public IntPtr Process, Thread;
            public uint ProcessId, ThreadId;
        }

        [DllImport("advapi32.dll", SetLastError = true)]
        private static extern bool SaferCreateLevel(
            uint scope, uint level, uint flags, out IntPtr handle, IntPtr reserved);

        [DllImport("advapi32.dll", SetLastError = true)]
        private static extern bool SaferComputeTokenFromLevel(
            IntPtr level, IntPtr input, out IntPtr token, uint flags, IntPtr reserved);

        [DllImport("advapi32.dll")]
        private static extern bool SaferCloseLevel(IntPtr level);

        [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool ConvertStringSidToSid(string value, out IntPtr sid);

        [DllImport("advapi32.dll")]
        private static extern uint GetLengthSid(IntPtr sid);

        [DllImport("advapi32.dll", SetLastError = true)]
        private static extern bool SetTokenInformation(
            IntPtr token, int informationClass, IntPtr information, uint length);

        [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool CreateProcessAsUser(
            IntPtr token, string application, StringBuilder commandLine,
            IntPtr processAttributes, IntPtr threadAttributes, bool inheritHandles,
            uint flags, IntPtr environment, string directory,
            ref StartupInfo startup, out ProcessInformation process);

        [DllImport("kernel32.dll")]
        private static extern bool CloseHandle(IntPtr handle);

        [DllImport("kernel32.dll")]
        private static extern IntPtr LocalFree(IntPtr memory);

        private static void Check(bool success)
        {
            if (!success)
                throw new Win32Exception(Marshal.GetLastWin32Error());
        }

        public static uint Start(string application, string commandLine, string directory)
        {
            IntPtr level = IntPtr.Zero;
            IntPtr token = IntPtr.Zero;
            IntPtr sid = IntPtr.Zero;
            IntPtr label = IntPtr.Zero;
            try
            {
                // SAFER_SCOPEID_MACHINE, SAFER_LEVELID_NORMALUSER, SAFER_LEVEL_OPEN.
                Check(SaferCreateLevel(1, 0x20000, 1, out level, IntPtr.Zero));
                Check(SaferComputeTokenFromLevel(
                    level, IntPtr.Zero, out token, 0, IntPtr.Zero));
                // runas /trustlevel keeps the caller's integrity level. Lower
                // the restricted token explicitly to SECURITY_MANDATORY_MEDIUM_RID.
                Check(ConvertStringSidToSid("S-1-16-8192", out sid));
                uint length = (uint)Marshal.SizeOf<MandatoryLabel>() + GetLengthSid(sid);
                label = Marshal.AllocHGlobal((int)length);
                Marshal.StructureToPtr(
                    new MandatoryLabel { Sid = sid, Attributes = 0x20 }, label, false);
                Check(SetTokenInformation(token, 25, label, length));

                var startup = new StartupInfo { Size = Marshal.SizeOf<StartupInfo>() };
                ProcessInformation process;
                // CREATE_NO_WINDOW hides the helper console. The application
                // under test still gets its normal desktop window.
                Check(CreateProcessAsUser(
                    token, application, new StringBuilder(commandLine),
                    IntPtr.Zero, IntPtr.Zero, false, 0x08000000, IntPtr.Zero,
                    directory, ref startup, out process));
                CloseHandle(process.Thread);
                CloseHandle(process.Process);
                return process.ProcessId;
            }
            finally
            {
                if (label != IntPtr.Zero) Marshal.FreeHGlobal(label);
                if (sid != IntPtr.Zero) LocalFree(sid);
                if (token != IntPtr.Zero) CloseHandle(token);
                if (level != IntPtr.Zero) SaferCloseLevel(level);
            }
        }
    }
}
