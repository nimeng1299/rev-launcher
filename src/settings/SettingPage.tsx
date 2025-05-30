import { Label, Select, useId } from "@fluentui/react-components";
import styles from "./SettingPage.module.css";
import JavaSetting from "./settingItems/JavaSetting";

/// modpack_id: global or modpack id
function SettingPage({ modpack_id = "global" }: { modpack_id?: string }) {
  const selectJavaId = useId("select-java-version");

  return (
    <div className={styles.main_div}>
      <div className={styles.flex}>
        <JavaSetting modpack_id={modpack_id} />
      </div>
    </div>
  );
}

export default SettingPage;
