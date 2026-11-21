$src = @'
using System; using System.Collections.Generic; using System.Runtime.InteropServices; using System.Text;
public static class KS {
  [DllImport("ntdll.dll")] static extern int NtQuerySystemInformation(int cls, IntPtr buf, int len, out int ret);
  public static Dictionary<string,long[]> Pool() {
    int len = 1<<20, ret; IntPtr b = Marshal.AllocHGlobal(len);
    try { int st; while ((st = NtQuerySystemInformation(22, b, len, out ret)) == unchecked((int)0xC0000004)) { Marshal.FreeHGlobal(b); len *= 2; b = Marshal.AllocHGlobal(len); }
      if (st != 0) throw new Exception("pool status 0x" + st.ToString("X"));
      int n = Marshal.ReadInt32(b); var d = new Dictionary<string,long[]>(); int sz = 40; long p = b.ToInt64() + 8;
      for (int i = 0; i < n; i++, p += sz) { byte[] t = new byte[4]; Marshal.Copy((IntPtr)p, t, 0, 4); string tag = Encoding.ASCII.GetString(t);
        long pa = (uint)Marshal.ReadInt32((IntPtr)(p+4)), pf = (uint)Marshal.ReadInt32((IntPtr)(p+8)), pu = Marshal.ReadInt64((IntPtr)(p+16));
        long na = (uint)Marshal.ReadInt32((IntPtr)(p+24)), nf = (uint)Marshal.ReadInt32((IntPtr)(p+28)), nu = Marshal.ReadInt64((IntPtr)(p+32));
        d[tag] = new long[]{ pa-pf, pu, na-nf, nu }; }
      return d; } finally { Marshal.FreeHGlobal(b); } }
  // SystemExtendedHandleInformation (64): count handles of pid by ObjectTypeIndex
  public static Dictionary<int,long> Handles(long pid) {
    int len = 1<<24, ret; IntPtr b = Marshal.AllocHGlobal(len);
    try { int st; while ((st = NtQuerySystemInformation(64, b, len, out ret)) == unchecked((int)0xC0000004)) { Marshal.FreeHGlobal(b); len = Math.Max(len*2, ret + (1<<20)); b = Marshal.AllocHGlobal(len); }
      if (st != 0) throw new Exception("handle status 0x" + st.ToString("X"));
      long n = Marshal.ReadInt64(b); var d = new Dictionary<int,long>(); long p = b.ToInt64() + 16; int sz = 40;
      for (long i = 0; i < n; i++, p += sz) { long upid = Marshal.ReadInt64((IntPtr)(p+8)); if (upid != pid) continue;
        int ti = (ushort)Marshal.ReadInt16((IntPtr)(p+30)); long c; d.TryGetValue(ti, out c); d[ti] = c+1; }
      return d; } finally { Marshal.FreeHGlobal(b); } }
}
'@
if (-not ("KS" -as [type])) { Add-Type -TypeDefinition $src }
