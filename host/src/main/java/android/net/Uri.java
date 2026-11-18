package android.net;

import java.net.URI;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/** The parts of Android's Uri that extensions read. Strings that don't parse still work as plain text. */
public class Uri {
    private final String s;
    private final URI u;

    private Uri(String s) {
        this.s = s;
        URI parsed;
        try { parsed = new URI(s); } catch (Exception e) { parsed = null; }
        u = parsed;
    }

    public static Uri parse(String s) { return new Uri(s); }
    public static String encode(String s) { return java.net.URLEncoder.encode(s, StandardCharsets.UTF_8).replace("+", "%20"); }

    public String getScheme() { return u == null ? null : u.getScheme(); }
    public String getHost() { return u == null ? null : u.getHost(); }
    public int getPort() { return u == null ? -1 : u.getPort(); }
    public String getAuthority() { return u == null ? null : u.getAuthority(); }
    public String getPath() { return u == null ? null : u.getPath(); }
    public String getEncodedPath() { return u == null ? null : u.getRawPath(); }
    public String getQuery() { return u == null ? null : u.getQuery(); }
    public String getEncodedQuery() { return u == null ? null : u.getRawQuery(); }
    public String getFragment() { return u == null ? null : u.getFragment(); }

    public List<String> getPathSegments() {
        List<String> out = new ArrayList<>();
        String p = getPath();
        if (p != null) for (String seg : p.split("/")) if (!seg.isEmpty()) out.add(seg);
        return out;
    }

    public String getLastPathSegment() {
        List<String> segs = getPathSegments();
        return segs.isEmpty() ? null : segs.get(segs.size() - 1);
    }

    public String getQueryParameter(String key) {
        String q = getEncodedQuery();
        if (q == null) return null;
        for (String pair : q.split("&")) {
            int eq = pair.indexOf('=');
            String k = URLDecoder.decode(eq < 0 ? pair : pair.substring(0, eq), StandardCharsets.UTF_8);
            if (k.equals(key)) return eq < 0 ? "" : URLDecoder.decode(pair.substring(eq + 1), StandardCharsets.UTF_8);
        }
        return null;
    }

    @Override public String toString() { return s; }
    @Override public boolean equals(Object o) { return o instanceof Uri other && other.s.equals(s); }
    @Override public int hashCode() { return s.hashCode(); }
}
