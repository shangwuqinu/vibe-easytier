package com.vibeeasytier.a14;

import android.Manifest;
import android.app.Activity;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.res.Configuration;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.net.VpnService;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.text.InputType;
import android.text.Editable;
import android.text.TextWatcher;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.ArrayAdapter;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.HorizontalScrollView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.Spinner;
import android.widget.Switch;
import android.widget.TextView;
import android.widget.Toast;

import com.easytier.jni.EasyTierJNI;
import com.vibeeasytier.a14.config.TomlProfileCodec;
import com.vibeeasytier.a14.core.PeerSnapshot;
import com.vibeeasytier.a14.model.EasyTierFlagSpec;
import com.vibeeasytier.a14.model.Profile;
import com.vibeeasytier.a14.model.ProfileValidator;
import com.vibeeasytier.a14.service.EasyTierVpnService;
import com.vibeeasytier.a14.storage.AppPreferences;
import com.vibeeasytier.a14.storage.LogStore;
import com.vibeeasytier.a14.storage.SecureProfileStore;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Date;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

public final class MainActivity extends Activity {
    private static final int REQUEST_VPN = 10;
    private static final int REQUEST_IMPORT = 11;
    private static final int REQUEST_EXPORT = 12;
    private static final int REQUEST_NOTIFICATIONS = 13;
    private static final int MAX_IMPORT_BYTES = 256 * 1024;

    private final List<String> navPages = List.of("概览", "私网", "节点", "日志", "设置");
    private final List<String> editingPeers = new ArrayList<>();
    private final List<PeerSnapshot> livePeers = new ArrayList<>();
    private final Map<String, Object> editingFlags = new LinkedHashMap<>();
    private final Map<String, View> flagInputs = new LinkedHashMap<>();
    private SecureProfileStore profileStore;
    private AppPreferences preferences;
    private LogStore logs;
    private FrameLayout content;
    private String currentPage = "概览";
    private Profile editingProfile;
    private String editingOriginalInstanceName;
    private EditText profileNameInput;
    private EditText instanceNameInput;
    private EditText hostnameInput;
    private EditText networkNameInput;
    private EditText secretInput;
    private EditText cidrInput;
    private EditText peerInput;
    private Spinner peerSpinner;
    private Spinner profileSpinner;
    private boolean receiverRegistered;
    private final BroadcastReceiver statusReceiver = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent intent) {
            ArrayList<String> peers = intent.getStringArrayListExtra(EasyTierVpnService.EXTRA_PEERS);
            livePeers.clear();
            if (peers != null) {
                for (String peer : peers) {
                    try {
                        livePeers.add(PeerSnapshot.fromTransportJson(peer));
                    } catch (Exception ignored) {
                        // A malformed telemetry row must not hide the rest of the status update.
                    }
                }
            }
            if (!"私网".equals(currentPage)) {
                renderPage(currentPage);
            }
        }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        profileStore = new SecureProfileStore(this);
        preferences = new AppPreferences(this);
        logs = new LogStore(this);
        loadProfile();
        setContentView(buildShell());
        registerStatusReceiver();
        requestNotificationPermission();
        renderPage(currentPage);
    }

    @Override
    protected void onResume() {
        super.onResume();
        if (content != null && !"私网".equals(currentPage)) {
            renderPage(currentPage);
        }
    }

    @Override
    protected void onDestroy() {
        if (receiverRegistered) {
            unregisterReceiver(statusReceiver);
        }
        super.onDestroy();
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQUEST_VPN) {
            if (resultCode == RESULT_OK) {
                preferences.setAutoConnect(true);
                startVpnService(EasyTierVpnService.connectIntent(this, true));
            } else {
                toast("未授予系统 VPN 权限");
            }
            renderPage("概览");
            return;
        }
        if (resultCode != RESULT_OK || data == null || data.getData() == null) {
            return;
        }
        if (requestCode == REQUEST_IMPORT) {
            importToml(data.getData());
        } else if (requestCode == REQUEST_EXPORT) {
            exportToml(data.getData());
        }
    }

    private View buildShell() {
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(backgroundColor());

        LinearLayout header = new LinearLayout(this);
        header.setOrientation(LinearLayout.VERTICAL);
        header.setPadding(dp(20), dp(14), dp(20), dp(12));
        TextView title = text("Vibe EasyTier", 21, true);
        TextView subtitle = text("Android 14 · EasyTier Core 2.6.4", 12, false);
        subtitle.setTextColor(mutedColor());
        header.addView(title);
        header.addView(subtitle);
        root.addView(header, new LinearLayout.LayoutParams(-1, -2));

        content = new FrameLayout(this);
        root.addView(content, new LinearLayout.LayoutParams(-1, 0, 1));

        HorizontalScrollView navScroll = new HorizontalScrollView(this);
        navScroll.setHorizontalScrollBarEnabled(false);
        navScroll.setFillViewport(true);
        navScroll.setBackgroundColor(surfaceColor());
        LinearLayout nav = new LinearLayout(this);
        nav.setOrientation(LinearLayout.HORIZONTAL);
        nav.setGravity(Gravity.CENTER);
        nav.setPadding(dp(6), dp(4), dp(6), dp(4));
        for (String page : navPages) {
            Button item = new Button(this);
            item.setText(page);
            item.setTextSize(13);
            item.setAllCaps(false);
            item.setTextColor(textColor());
            item.setBackgroundColor(Color.TRANSPARENT);
            item.setMinHeight(dp(48));
            item.setOnClickListener(view -> renderPage(page));
            nav.addView(item, new LinearLayout.LayoutParams(dp(74), dp(52)));
        }
        navScroll.addView(nav, new ViewGroup.LayoutParams(-2, -1));
        root.addView(navScroll, new LinearLayout.LayoutParams(-1, dp(60)));
        return root;
    }

    private void renderPage(String page) {
        currentPage = page;
        content.removeAllViews();
        View view = switch (page) {
            case "私网" -> networkPage();
            case "节点" -> nodesPage();
            case "日志" -> logsPage();
            case "设置" -> settingsPage();
            default -> overviewPage();
        };
        content.addView(view, new FrameLayout.LayoutParams(-1, -1));
    }

    private View overviewPage() {
        LinearLayout page = pageColumn();
        page.addView(pageTitle("概览", "连接与自动恢复状态"));
        AppPreferences.Status status = preferences.readStatus();

        LinearLayout connection = card();
        TextView state = text(stateLabel(status.state()), 22, true);
        state.setTextColor(stateColor(status.state()));
        connection.addView(state);
        TextView detail = text(status.detail(), 14, false);
        detail.setTextColor(mutedColor());
        connection.addView(detail);
        addMetric(connection, "远端节点", Integer.toString(status.peerCount()));
        addMetric(connection, "路由", Integer.toString(status.routeCount()));
        addMetric(connection, "已发送", formatBytes(status.sentBytes()));
        addMetric(connection, "已接收", formatBytes(status.receivedBytes()));
        addMetric(connection, "虚拟地址", editingProfile == null ? "未配置" : editingProfile.ipv4Cidr());
        page.addView(connection, cardParams());

        LinearLayout startup = card();
        startup.addView(sectionTitle("自动连接"));
        addMetric(startup, "连接意图", preferences.autoConnect() ? "已启用" : "已关闭");
        addMetric(startup, "系统 VPN 授权", VpnService.prepare(this) == null ? "已授权" : "待授权");
        addMetric(startup, "Always-on VPN", preferences.alwaysOn() ? "已启用" : "可在系统设置启用");
        addMetric(startup, "最近连接成功", status.lastSuccess() == 0
                ? "尚无记录" : formatDateTime(status.lastSuccess()));
        if (status.retryAt() > System.currentTimeMillis()) {
            addMetric(startup, "下次重试", formatTime(status.retryAt()));
        }
        page.addView(startup, cardParams());

        LinearLayout actions = horizontal();
        actions.addView(commandButton("连接", true, view -> connect()), weightParams());
        actions.addView(commandButton("断开", false, view -> disconnect()), weightParams());
        page.addView(actions, new LinearLayout.LayoutParams(-1, -2));
        return scroll(page);
    }

    private View networkPage() {
        LinearLayout page = pageColumn();
        page.addView(pageTitle("私有网络", "固定地址与 Bootstrap 节点"));
        Profile profile = editingProfile == null ? Profile.empty(defaultHostname()) : editingProfile;
        if (editingPeers.isEmpty() && !profile.peers().isEmpty()) {
            editingPeers.addAll(profile.peers());
        }
        if (editingFlags.isEmpty()) {
            editingFlags.putAll(profile.flags());
        }

        LinearLayout catalog = card();
        catalog.addView(sectionTitle("档案管理"));
        try {
            List<Profile> profiles = profileStore.list();
            List<String> names = new ArrayList<>();
            int selected = 0;
            for (int index = 0; index < profiles.size(); index++) {
                Profile item = profiles.get(index);
                names.add(item.profileName() + " · " + item.instanceName());
                if (editingOriginalInstanceName != null
                        && editingOriginalInstanceName.equals(item.instanceName())) {
                    selected = index;
                }
            }
            profileSpinner = new Spinner(this);
            profileSpinner.setAdapter(new ArrayAdapter<>(this,
                    android.R.layout.simple_spinner_dropdown_item,
                    names.isEmpty() ? List.of("尚未保存档案") : names));
            profileSpinner.setSelection(selected);
            profileSpinner.setEnabled(!names.isEmpty());
            catalog.addView(profileSpinner, new LinearLayout.LayoutParams(-1, dp(52)));
            LinearLayout catalogActions = horizontal();
            catalogActions.addView(commandButton("切换", false, view -> selectProfile()), weightParams());
            catalogActions.addView(commandButton("新建", false, view -> newProfile()), weightParams());
            catalogActions.addView(commandButton("删除", false, view -> deleteProfile()), weightParams());
            catalog.addView(catalogActions);
        } catch (Exception error) {
            TextView detail = text("读取档案列表失败：" + safeMessage(error), 13, false);
            detail.setTextColor(Color.parseColor("#D24B4B"));
            catalog.addView(detail);
        }
        page.addView(catalog, cardParams());

        LinearLayout form = card();
        profileNameInput = field(form, "档案名称", profile.profileName(), false);
        instanceNameInput = field(form, "实例名称", profile.instanceName(), false);
        hostnameInput = field(form, "设备名称（留空使用本机型号）", profile.hostname(), false);
        networkNameInput = field(form, "网络名称", profile.networkName(), false);
        secretInput = field(form, "网络密钥", profile.networkSecret(), true);
        cidrInput = field(form, "固定虚拟 IPv4/CIDR", profile.ipv4Cidr(), false);
        page.addView(form, cardParams());

        LinearLayout peers = card();
        peers.addView(sectionTitle("Bootstrap 节点"));
        peerInput = field(peers, "节点 URI", "", false);
        LinearLayout peerActions = horizontal();
        peerActions.addView(commandButton("添加", true, view -> addPeers()), weightParams());
        peerActions.addView(commandButton("移除所选", false, view -> removeSelectedPeer()), weightParams());
        peers.addView(peerActions);
        peerSpinner = new Spinner(this);
        peerSpinner.setMinimumHeight(dp(48));
        refreshPeerSpinner();
        peers.addView(peerSpinner, new LinearLayout.LayoutParams(-1, dp(52)));
        page.addView(peers, cardParams());

        LinearLayout actions = horizontal();
        actions.addView(commandButton("保存档案", true, view -> saveProfile()), weightParams());
        actions.addView(commandButton("导入 TOML", false, view -> pickImport()), weightParams());
        page.addView(actions);
        page.addView(commandButton("导出 TOML", false, view -> pickExport()), new LinearLayout.LayoutParams(-1, dp(48)));
        return scroll(page);
    }

    private View nodesPage() {
        LinearLayout page = pageColumn();
        page.addView(pageTitle("节点", "远端路由与活动传输协议"));
        AppPreferences.Status status = preferences.readStatus();
        LinearLayout summary = card();
        addMetric(summary, "远端节点", Integer.toString(status.peerCount()));
        addMetric(summary, "连接状态", stateLabel(status.state()));
        page.addView(summary, cardParams());
        addMetric(summary, "路由", Integer.toString(status.routeCount()));
        addMetric(summary, "已发送", formatBytes(status.sentBytes()));
        addMetric(summary, "已接收", formatBytes(status.receivedBytes()));
        if (livePeers.isEmpty()) {
            LinearLayout empty = card();
            TextView label = text("尚未发现远端节点", 15, true);
            empty.addView(label);
            TextView detail = text("Core 建立路由后将在此显示节点与 TCP、UDP、WireGuard 等活动协议。", 13, false);
            detail.setTextColor(mutedColor());
            empty.addView(detail);
            page.addView(empty, cardParams());
        } else {
            for (PeerSnapshot node : livePeers) {
                LinearLayout item = card();
                item.addView(text(node.displayName(), 16, true));
                addMetric(item, "状态", "在线");
                addMetric(item, "虚拟地址", node.virtualIp().isBlank() ? "未知" : node.virtualIp());
                addMetric(item, "角色", node.relay() ? "中继路由" : "直连节点");
                addMetric(item, "连接协议", node.protocols().isEmpty()
                        ? "未报告" : String.join(" / ", node.protocols()));
                addMetric(item, "延迟", node.latencyMs() == 0 ? "未报告" : node.latencyMs() + " ms");
                addMetric(item, "版本", node.version().isBlank() ? "未报告" : node.version());
                addMetric(item, "最近出现", "刚刚");
                addMetric(item, "已发送", formatBytes(node.sentBytes()));
                addMetric(item, "已接收", formatBytes(node.receivedBytes()));
                page.addView(item, cardParams());
            }
        }
        return scroll(page);
    }

    private View logsPage() {
        LinearLayout page = pageColumn();
        LinearLayout titleRow = horizontal();
        titleRow.addView(pageTitle("日志", "最近 200 条运行事件"), weightParams());
        titleRow.addView(commandButton("清空", false, view -> {
            logs.clear();
            renderPage("日志");
        }), new LinearLayout.LayoutParams(dp(80), dp(48)));
        page.addView(titleRow);
        EditText search = new EditText(this);
        search.setHint("搜索日志");
        search.setSingleLine(true);
        search.setTextSize(14);
        search.setTextColor(textColor());
        search.setHintTextColor(mutedColor());
        search.setBackgroundTintList(ColorStateList.valueOf(accentColor()));
        page.addView(search, new LinearLayout.LayoutParams(-1, dp(48)));
        LinearLayout logCard = card();
        List<String> lines = logs.read();
        TextView output = text(lines.isEmpty() ? "暂无日志" : String.join("\n\n", lines), 13, false);
        output.setTypeface(Typeface.MONOSPACE);
        output.setTextIsSelectable(true);
        output.setHorizontallyScrolling(false);
        output.setLineSpacing(0, 1.15f);
        logCard.addView(output);
        search.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence value, int start, int count, int after) {}
            @Override public void onTextChanged(CharSequence value, int start, int before, int count) {
                String query = value.toString().trim().toLowerCase(Locale.ROOT);
                List<String> filtered = query.isEmpty() ? lines : lines.stream()
                        .filter(line -> line.toLowerCase(Locale.ROOT).contains(query))
                        .toList();
                output.setText(filtered.isEmpty() ? "没有匹配的日志" : String.join("\n\n", filtered));
            }
            @Override public void afterTextChanged(Editable value) {}
        });
        page.addView(logCard, cardParams());
        return scroll(page);
    }

    private View settingsPage() {
        LinearLayout page = pageColumn();
        page.addView(pageTitle("设置", "系统接管与 Core 参数"));
        LinearLayout behavior = card();
        Switch autoConnect = new Switch(this);
        autoConnect.setText("自动连接私有网络");
        autoConnect.setTextColor(textColor());
        autoConnect.setTextSize(15);
        autoConnect.setChecked(preferences.autoConnect());
        autoConnect.setPadding(0, dp(6), 0, dp(6));
        autoConnect.setOnCheckedChangeListener((button, checked) -> {
            if (checked) {
                connect();
            } else {
                disconnect();
            }
        });
        behavior.addView(autoConnect, new LinearLayout.LayoutParams(-1, dp(56)));
        behavior.addView(commandButton("打开系统 VPN 设置", false, view ->
                startActivity(new Intent(Settings.ACTION_VPN_SETTINGS))), new LinearLayout.LayoutParams(-1, dp(48)));
        behavior.addView(commandButton("打开电池优化设置", false, view ->
                startActivity(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))),
                new LinearLayout.LayoutParams(-1, dp(48)));
        TextView themeLabel = text("界面主题", 13, true);
        themeLabel.setPadding(0, dp(12), 0, dp(4));
        behavior.addView(themeLabel);
        Spinner theme = new Spinner(this);
        List<String> themeLabels = List.of("跟随系统", "浅色", "深色");
        theme.setAdapter(new ArrayAdapter<>(this, android.R.layout.simple_spinner_dropdown_item, themeLabels));
        theme.setSelection(switch (preferences.theme()) {
            case "light" -> 1;
            case "dark" -> 2;
            default -> 0;
        });
        behavior.addView(theme, new LinearLayout.LayoutParams(-1, dp(50)));
        behavior.addView(commandButton("应用界面主题", false, view -> {
            String value = switch (theme.getSelectedItemPosition()) {
                case 1 -> "light";
                case 2 -> "dark";
                default -> "system";
            };
            preferences.setTheme(value);
            recreate();
        }), new LinearLayout.LayoutParams(-1, dp(48)));
        page.addView(behavior, cardParams());

        flagInputs.clear();
        for (EasyTierFlagSpec.Section section : EasyTierFlagSpec.SECTIONS) {
            LinearLayout flags = card();
            flags.addView(sectionTitle(section.title()));
            TextView description = text(section.description(), 12, false);
            description.setTextColor(mutedColor());
            description.setPadding(0, 0, 0, dp(6));
            flags.addView(description);
            for (EasyTierFlagSpec.Field field : section.fields()) {
                addFlagField(flags, field);
            }
            page.addView(flags, cardParams());
        }
        page.addView(commandButton("保存全部核心设置", true, view -> saveFlagSettings()),
                new LinearLayout.LayoutParams(-1, dp(50)));

        LinearLayout core = card();
        core.addView(sectionTitle("运行组件"));
        addMetric(core, "EasyTier Core", EasyTierJNI.isAvailable() ? "2.6.4 已就绪" : "2.6.4 未暂存");
        addMetric(core, "目标系统", "Android 14 / API 34");
        addMetric(core, "目标架构", "arm64-v8a");
        page.addView(core, cardParams());
        return scroll(page);
    }

    private void addFlagField(LinearLayout parent, EasyTierFlagSpec.Field field) {
        if (field.kind() == EasyTierFlagSpec.Kind.BOOLEAN) {
            CheckBox check = new CheckBox(this);
            check.setText(field.label());
            check.setTextColor(textColor());
            check.setTextSize(14);
            Object value = field.locked() ? field.lockedValue() : editingFlags.get(field.key());
            check.setChecked(Boolean.TRUE.equals(value));
            check.setEnabled(!field.locked());
            parent.addView(check, new LinearLayout.LayoutParams(-1, dp(46)));
            flagInputs.put(field.key(), check);
        } else if (field.kind() == EasyTierFlagSpec.Kind.SELECT) {
            TextView label = text(field.label(), 13, true);
            label.setPadding(0, dp(8), 0, 0);
            parent.addView(label);
            List<String> labels = field.options().stream().map(EasyTierFlagSpec.Option::label).toList();
            Spinner spinner = new Spinner(this);
            spinner.setAdapter(new ArrayAdapter<>(this, android.R.layout.simple_spinner_dropdown_item, labels));
            Object current = editingFlags.get(field.key());
            for (int index = 0; index < field.options().size(); index++) {
                if (String.valueOf(field.options().get(index).value()).equals(String.valueOf(current))) {
                    spinner.setSelection(index);
                    break;
                }
            }
            spinner.setEnabled(!field.locked());
            parent.addView(spinner, new LinearLayout.LayoutParams(-1, dp(48)));
            flagInputs.put(field.key(), spinner);
        } else {
            TextView label = text(field.label(), 13, true);
            label.setPadding(0, dp(8), 0, 0);
            parent.addView(label);
            EditText input = new EditText(this);
            Object current = field.locked() ? field.lockedValue() : editingFlags.get(field.key());
            input.setText(current == null ? "" : String.valueOf(current));
            input.setSingleLine(true);
            input.setTextSize(14);
            input.setTextColor(textColor());
            input.setEnabled(!field.locked());
            if (field.kind() == EasyTierFlagSpec.Kind.NUMBER || field.kind() == EasyTierFlagSpec.Kind.RATE) {
                input.setInputType(InputType.TYPE_CLASS_NUMBER);
            }
            input.setBackgroundTintList(ColorStateList.valueOf(accentColor()));
            parent.addView(input, new LinearLayout.LayoutParams(-1, dp(46)));
            flagInputs.put(field.key(), input);
        }
        TextView help = text(field.description(), 12, false);
        help.setTextColor(mutedColor());
        help.setPadding(0, 0, 0, dp(5));
        parent.addView(help);
    }

    private void collectFlagInputs() {
        for (EasyTierFlagSpec.Section section : EasyTierFlagSpec.SECTIONS) {
            for (EasyTierFlagSpec.Field field : section.fields()) {
                View input = flagInputs.get(field.key());
                if (field.locked()) {
                    editingFlags.put(field.key(), field.lockedValue());
                } else if (input instanceof CheckBox check) {
                    editingFlags.put(field.key(), check.isChecked());
                } else if (input instanceof Spinner spinner) {
                    int index = spinner.getSelectedItemPosition();
                    editingFlags.put(field.key(), field.options().get(index).value());
                } else if (input instanceof EditText edit) {
                    String value = edit.getText().toString().trim();
                    if (field.kind() == EasyTierFlagSpec.Kind.RATE && value.isEmpty()) {
                        editingFlags.remove(field.key());
                    } else if (field.kind() == EasyTierFlagSpec.Kind.NUMBER
                            || field.kind() == EasyTierFlagSpec.Kind.RATE) {
                        if (value.isEmpty()) {
                            throw new IllegalArgumentException(field.label() + "不能为空");
                        }
                        try {
                            editingFlags.put(field.key(), Long.parseLong(value));
                        } catch (NumberFormatException error) {
                            throw new IllegalArgumentException(field.label() + "必须是有效整数");
                        }
                    } else {
                        editingFlags.put(field.key(), value);
                    }
                }
            }
        }
    }

    private void connect() {
        if (!profileStore.exists()) {
            toast("请先保存私有网络档案");
            renderPage("私网");
            return;
        }
        Intent prepare = VpnService.prepare(this);
        if (prepare != null) {
            startActivityForResult(prepare, REQUEST_VPN);
            return;
        }
        preferences.setAutoConnect(true);
        startVpnService(EasyTierVpnService.connectIntent(this, true));
        renderPage("概览");
    }

    private void disconnect() {
        if (preferences.alwaysOn()) {
            toast("Always-on VPN 由系统管理，请在系统 VPN 设置中关闭");
            startActivity(new Intent(Settings.ACTION_VPN_SETTINGS));
            return;
        }
        preferences.setAutoConnect(false);
        startService(EasyTierVpnService.stopIntent(this));
        renderPage("概览");
    }

    private void saveProfile() {
        try {
            String hostname = hostnameInput.getText().toString().trim();
            if (hostname.isEmpty()) {
                hostname = defaultHostname();
            }
            Map<String, Object> flags = editingFlags.isEmpty()
                    ? Profile.defaultFlags() : new LinkedHashMap<>(editingFlags);
            flags.put("enable_encryption", true);
            flags.put("private_mode", true);
            flags.put("no_tun", false);
            flags.put("enable_exit_node", false);
            Profile profile = new Profile(
                    profileNameInput.getText().toString(),
                    instanceNameInput.getText().toString(),
                    hostname,
                    networkNameInput.getText().toString(),
                    secretInput.getText().toString(),
                    cidrInput.getText().toString(),
                    editingPeers,
                    flags);
            ProfileValidator.validate(profile);
            validateWithCore(profile);
            profileStore.save(editingOriginalInstanceName, profile);
            editingProfile = profile;
            editingOriginalInstanceName = profile.instanceName();
            logs.append("私有网络档案已安全保存");
            toast("档案已保存");
            if (preferences.autoConnect()) {
                startVpnService(new Intent(this, EasyTierVpnService.class)
                        .setAction(EasyTierVpnService.ACTION_RELOAD));
            }
        } catch (Exception error) {
            toast("保存失败：" + safeMessage(error));
        }
    }

    private void saveFlagSettings() {
        if (editingProfile == null || !profileStore.exists()) {
            toast("请先保存私有网络档案");
            renderPage("私网");
            return;
        }
        try {
            collectFlagInputs();
            Map<String, Object> flags = new LinkedHashMap<>(editingFlags);
            flags.put("enable_encryption", true);
            flags.put("private_mode", true);
            flags.put("no_tun", false);
            flags.put("enable_exit_node", false);
            Profile updated = new Profile(
                    editingProfile.profileName(), editingProfile.instanceName(), editingProfile.hostname(),
                    editingProfile.networkName(), editingProfile.networkSecret(), editingProfile.ipv4Cidr(),
                    editingProfile.peers(), flags);
            ProfileValidator.validate(updated);
            validateWithCore(updated);
            profileStore.save(editingOriginalInstanceName, updated);
            editingProfile = updated;
            editingOriginalInstanceName = updated.instanceName();
            logs.append("核心设置已安全保存");
            toast("核心设置已保存");
            if (preferences.autoConnect()) {
                startVpnService(new Intent(this, EasyTierVpnService.class)
                        .setAction(EasyTierVpnService.ACTION_RELOAD));
            }
        } catch (Exception error) {
            toast("保存失败：" + safeMessage(error));
        }
    }

    private void addPeers() {
        String source = peerInput.getText().toString();
        String[] candidates = source.split("[\\s,，]+", -1);
        try {
            for (String candidate : candidates) {
                String value = candidate.trim();
                if (value.isEmpty() || editingPeers.contains(value)) {
                    continue;
                }
                List<String> probePeers = new ArrayList<>();
                probePeers.add(value);
                Profile probe = new Profile(
                        "probe", "probe", "Android", "probe", "probe", "100.64.0.1/24",
                        probePeers, Profile.defaultFlags());
                ProfileValidator.validate(probe);
                if (editingPeers.size() >= 8) {
                    throw new IllegalArgumentException("Bootstrap 节点最多 8 个");
                }
                editingPeers.add(value);
            }
            peerInput.setText("");
            refreshPeerSpinner();
        } catch (Exception error) {
            toast("添加失败：" + safeMessage(error));
        }
    }

    private void removeSelectedPeer() {
        if (peerSpinner == null || editingPeers.isEmpty()) {
            return;
        }
        int selected = peerSpinner.getSelectedItemPosition();
        if (selected >= 0 && selected < editingPeers.size()) {
            editingPeers.remove(selected);
            refreshPeerSpinner();
        }
    }

    private void refreshPeerSpinner() {
        if (peerSpinner == null) {
            return;
        }
        List<String> display = editingPeers.isEmpty() ? List.of("尚未添加") : editingPeers;
        ArrayAdapter<String> adapter = new ArrayAdapter<>(
                this, android.R.layout.simple_spinner_dropdown_item, display);
        peerSpinner.setAdapter(adapter);
    }

    private void selectProfile() {
        try {
            List<Profile> profiles = profileStore.list();
            if (profiles.isEmpty() || profileSpinner == null) {
                toast("尚未保存档案");
                return;
            }
            int selected = profileSpinner.getSelectedItemPosition();
            if (selected < 0 || selected >= profiles.size()) {
                throw new IllegalArgumentException("请选择有效档案");
            }
            profileStore.select(profiles.get(selected).instanceName());
            loadProfile();
            logs.append("已切换活动私有网络档案");
            if (preferences.autoConnect()) {
                startVpnService(new Intent(this, EasyTierVpnService.class)
                        .setAction(EasyTierVpnService.ACTION_RELOAD));
            }
            renderPage("私网");
        } catch (Exception error) {
            toast("切换失败：" + safeMessage(error));
        }
    }

    private void newProfile() {
        editingProfile = Profile.empty(defaultHostname());
        editingOriginalInstanceName = null;
        editingPeers.clear();
        editingFlags.clear();
        editingFlags.putAll(editingProfile.flags());
        renderPage("私网");
    }

    private void deleteProfile() {
        if (editingOriginalInstanceName == null || !profileStore.exists()) {
            toast("当前档案尚未保存");
            return;
        }
        try {
            String removed = editingOriginalInstanceName;
            if (preferences.alwaysOn() && profileStore.list().size() == 1) {
                toast("请先在系统 VPN 设置中关闭 Always-on VPN，再删除最后一个档案");
                startActivity(new Intent(Settings.ACTION_VPN_SETTINGS));
                return;
            }
            profileStore.delete(removed);
            loadProfile();
            logs.append("已删除私有网络档案：" + removed);
            if (preferences.autoConnect()) {
                if (profileStore.exists()) {
                    startVpnService(new Intent(this, EasyTierVpnService.class)
                            .setAction(EasyTierVpnService.ACTION_RELOAD));
                } else {
                    preferences.setAutoConnect(false);
                    startService(EasyTierVpnService.stopIntent(this));
                }
            }
            renderPage("私网");
        } catch (Exception error) {
            toast("删除失败：" + safeMessage(error));
        }
    }

    private void pickImport() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                .addCategory(Intent.CATEGORY_OPENABLE)
                .setType("*/*");
        startActivityForResult(intent, REQUEST_IMPORT);
    }

    private void pickExport() {
        if (!profileStore.exists()) {
            toast("尚未保存档案");
            return;
        }
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT)
                .addCategory(Intent.CATEGORY_OPENABLE)
                .setType("application/toml")
                .putExtra(Intent.EXTRA_TITLE, "vibe-easytier.toml");
        startActivityForResult(intent, REQUEST_EXPORT);
    }

    private void importToml(Uri uri) {
        try (InputStream input = getContentResolver().openInputStream(uri);
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            if (input == null) {
                throw new IllegalArgumentException("无法读取所选文件");
            }
            byte[] buffer = new byte[8192];
            int total = 0;
            int read;
            while ((read = input.read(buffer)) != -1) {
                total += read;
                if (total > MAX_IMPORT_BYTES) {
                    throw new IllegalArgumentException("TOML 文件不能超过 256 KiB");
                }
                output.write(buffer, 0, read);
            }
            Profile profile = TomlProfileCodec.parse(
                    new String(output.toByteArray(), StandardCharsets.UTF_8), defaultHostname());
            validateWithCore(profile);
            profileStore.save(profile.instanceName(), profile);
            editingProfile = profile;
            editingOriginalInstanceName = profile.instanceName();
            editingPeers.clear();
            editingPeers.addAll(profile.peers());
            editingFlags.clear();
            editingFlags.putAll(profile.flags());
            logs.append("已从本地文件导入私有网络档案");
            toast("TOML 已导入并保存");
            if (preferences.autoConnect()) {
                startVpnService(new Intent(this, EasyTierVpnService.class)
                        .setAction(EasyTierVpnService.ACTION_RELOAD));
            }
            renderPage("私网");
        } catch (Exception error) {
            toast("导入失败：" + safeMessage(error));
        }
    }

    private void exportToml(Uri uri) {
        try (OutputStream output = getContentResolver().openOutputStream(uri, "wt")) {
            if (output == null) {
                throw new IllegalArgumentException("无法写入所选文件");
            }
            Profile profile = profileStore.load();
            output.write(TomlProfileCodec.render(profile).getBytes(StandardCharsets.UTF_8));
            logs.append("已将私有网络档案导出为 TOML");
            toast("TOML 已导出");
        } catch (Exception error) {
            toast("导出失败：" + safeMessage(error));
        }
    }

    private void loadProfile() {
        try {
            editingProfile = profileStore.exists() ? profileStore.load() : Profile.empty(defaultHostname());
            editingOriginalInstanceName = profileStore.exists() ? editingProfile.instanceName() : null;
            editingPeers.clear();
            editingPeers.addAll(editingProfile.peers());
            editingFlags.clear();
            editingFlags.putAll(editingProfile.flags());
        } catch (Exception error) {
            editingProfile = Profile.empty(defaultHostname());
            editingOriginalInstanceName = null;
            editingPeers.clear();
            editingFlags.clear();
            editingFlags.putAll(editingProfile.flags());
            logs.append("读取加密档案失败，未覆盖原文件：" + safeMessage(error));
        }
    }

    private void validateWithCore(Profile profile) {
        try {
            if (!EasyTierJNI.isAvailable()) {
                throw new IllegalStateException("内置 EasyTier Core 2.6.4 未暂存");
            }
            int result = EasyTierJNI.parseConfig(TomlProfileCodec.render(profile));
            if (result != 0) {
                String detail = EasyTierJNI.getLastError();
                if (detail == null || detail.isBlank()) {
                    detail = "Core 拒绝该配置";
                }
                throw new IllegalArgumentException(detail.replace(profile.networkSecret(), "[已隐藏]"));
            }
        } catch (RuntimeException error) {
            String detail = error.getMessage();
            if (detail == null || detail.isBlank()) {
                detail = "Core 配置校验失败";
            }
            throw new IllegalArgumentException(
                    detail.replace(profile.networkSecret(), "[已隐藏]"), error);
        } catch (LinkageError error) {
            throw new IllegalStateException("内置 EasyTier Core 2.6.4 无法加载", error);
        }
    }

    private void registerStatusReceiver() {
        IntentFilter filter = new IntentFilter(EasyTierVpnService.ACTION_STATUS);
        registerReceiver(
                statusReceiver,
                filter,
                EasyTierVpnService.INTERNAL_STATUS_PERMISSION,
                null,
                Context.RECEIVER_NOT_EXPORTED);
        receiverRegistered = true;
    }

    private void requestNotificationPermission() {
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != getPackageManager().PERMISSION_GRANTED) {
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, REQUEST_NOTIFICATIONS);
        }
    }

    private void startVpnService(Intent intent) {
        startForegroundService(intent);
    }

    private LinearLayout pageColumn() {
        LinearLayout page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        page.setPadding(dp(16), dp(8), dp(16), dp(24));
        return page;
    }

    private ScrollView scroll(View child) {
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setClipToPadding(false);
        scroll.setVerticalScrollBarEnabled(true);
        scroll.addView(child, new ScrollView.LayoutParams(-1, -2));
        return scroll;
    }

    private LinearLayout card() {
        LinearLayout card = new LinearLayout(this);
        card.setOrientation(LinearLayout.VERTICAL);
        card.setPadding(dp(16), dp(14), dp(16), dp(14));
        card.setBackground(rounded(surfaceColor(), 8));
        return card;
    }

    private View pageTitle(String title, String subtitle) {
        LinearLayout block = new LinearLayout(this);
        block.setOrientation(LinearLayout.VERTICAL);
        block.setPadding(dp(2), dp(4), dp(2), dp(12));
        block.addView(text(title, 22, true));
        TextView detail = text(subtitle, 13, false);
        detail.setTextColor(mutedColor());
        block.addView(detail);
        return block;
    }

    private TextView sectionTitle(String value) {
        TextView title = text(value, 16, true);
        title.setPadding(0, 0, 0, dp(8));
        return title;
    }

    private void addMetric(LinearLayout parent, String label, String value) {
        LinearLayout row = horizontal();
        row.setPadding(0, dp(7), 0, dp(7));
        TextView key = text(label, 14, false);
        key.setTextColor(mutedColor());
        TextView content = text(value, 14, true);
        content.setGravity(Gravity.END);
        row.addView(key, weightParams());
        row.addView(content, weightParams());
        parent.addView(row);
    }

    private EditText field(LinearLayout parent, String label, String value, boolean secret) {
        TextView caption = text(label, 13, true);
        caption.setPadding(0, dp(7), 0, dp(3));
        parent.addView(caption);
        EditText input = new EditText(this);
        input.setText(value);
        input.setTextSize(15);
        input.setTextColor(textColor());
        input.setSingleLine(true);
        input.setSelectAllOnFocus(false);
        input.setBackgroundTintList(ColorStateList.valueOf(accentColor()));
        if (secret) {
            input.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD);
        }
        parent.addView(input, new LinearLayout.LayoutParams(-1, dp(50)));
        return input;
    }

    private Button commandButton(String label, boolean primary, View.OnClickListener listener) {
        Button button = new Button(this);
        button.setText(label);
        button.setAllCaps(false);
        button.setTextSize(14);
        button.setTextColor(primary ? Color.WHITE : textColor());
        button.setBackground(rounded(primary ? accentColor() : secondaryButtonColor(), 7));
        button.setOnClickListener(listener);
        button.setMinHeight(dp(44));
        return button;
    }

    private LinearLayout horizontal() {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        row.setGravity(Gravity.CENTER_VERTICAL);
        return row;
    }

    private TextView text(String value, float size, boolean bold) {
        TextView text = new TextView(this);
        text.setText(value);
        text.setTextSize(size);
        text.setTextColor(textColor());
        text.setTypeface(Typeface.DEFAULT, bold ? Typeface.BOLD : Typeface.NORMAL);
        text.setLetterSpacing(0);
        return text;
    }

    private GradientDrawable rounded(int color, int radiusDp) {
        GradientDrawable drawable = new GradientDrawable();
        drawable.setColor(color);
        drawable.setCornerRadius(dp(radiusDp));
        return drawable;
    }

    private LinearLayout.LayoutParams cardParams() {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
        params.setMargins(0, 0, 0, dp(10));
        return params;
    }

    private LinearLayout.LayoutParams weightParams() {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(0, -2, 1);
        params.setMargins(dp(3), 0, dp(3), 0);
        return params;
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    private boolean isDark() {
        if (preferences != null) {
            if ("dark".equals(preferences.theme())) {
                return true;
            }
            if ("light".equals(preferences.theme())) {
                return false;
            }
        }
        return (getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK)
                == Configuration.UI_MODE_NIGHT_YES;
    }

    private int backgroundColor() { return Color.parseColor(isDark() ? "#101419" : "#F4F6F8"); }
    private int surfaceColor() { return Color.parseColor(isDark() ? "#1B2128" : "#FFFFFF"); }
    private int textColor() { return Color.parseColor(isDark() ? "#EDF2F7" : "#17202A"); }
    private int mutedColor() { return Color.parseColor(isDark() ? "#A8B3BF" : "#66717E"); }
    private int accentColor() { return Color.parseColor(isDark() ? "#5DA9F6" : "#2878D0"); }
    private int secondaryButtonColor() { return Color.parseColor(isDark() ? "#2A333D" : "#E7ECF1"); }

    private int stateColor(String state) {
        return switch (state) {
            case "CONNECTED" -> Color.parseColor("#2E9B64");
            case "FAILED" -> Color.parseColor("#D24B4B");
            case "RECOVERING", "CONNECTING" -> Color.parseColor("#D98324");
            default -> mutedColor();
        };
    }

    private String stateLabel(String state) {
        return switch (state) {
            case "CONNECTED" -> "已连接";
            case "CONNECTING" -> "正在连接";
            case "RECOVERING" -> "正在恢复";
            case "FAILED" -> "连接失败";
            default -> "已断开";
        };
    }

    private String defaultHostname() {
        String model = Build.MODEL == null ? "Android" : Build.MODEL.trim();
        return model.isEmpty() ? "Android" : model;
    }

    private String formatTime(long millis) {
        return new SimpleDateFormat("HH:mm:ss", Locale.CHINA).format(new Date(millis));
    }

    private String formatDateTime(long millis) {
        return new SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.CHINA).format(new Date(millis));
    }

    private String formatBytes(long bytes) {
        if (bytes < 1024) {
            return bytes + " B";
        }
        double value = bytes;
        String[] units = {"KiB", "MiB", "GiB", "TiB"};
        int unit = -1;
        do {
            value /= 1024.0;
            unit++;
        } while (value >= 1024 && unit < units.length - 1);
        return String.format(Locale.CHINA, "%.1f %s", value, units[unit]);
    }

    private String safeMessage(Throwable error) {
        String message = error.getMessage();
        return message == null || message.isBlank() ? error.getClass().getSimpleName()
                : message.replace('\n', ' ').replace('\r', ' ');
    }

    private void toast(String message) {
        Toast.makeText(this, message, Toast.LENGTH_LONG).show();
    }
}
