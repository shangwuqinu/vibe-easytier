package com.easytier.jni;

public final class EasyTierJNI {
    private static final boolean AVAILABLE;
    private static final String LOAD_ERROR;

    static {
        boolean available = false;
        String error = "";
        try {
            System.loadLibrary("easytier_ffi");
            System.loadLibrary("easytier_android_jni");
            available = true;
        } catch (UnsatisfiedLinkError loadError) {
            error = loadError.getMessage() == null ? "native library unavailable" : loadError.getMessage();
        }
        AVAILABLE = available;
        LOAD_ERROR = error;
    }

    private EasyTierJNI() {}

    public static boolean isAvailable() { return AVAILABLE; }
    public static String loadError() { return LOAD_ERROR; }

    public static native int setTunFd(String instanceName, int fd);
    public static native int parseConfig(String config);
    public static native int runNetworkInstance(String config);
    public static native int retainNetworkInstance(String[] instanceNames);
    public static native String collectNetworkInfos();
    public static native String getLastError();
}

