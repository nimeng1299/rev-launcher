import { Label, Select, useId } from "@fluentui/react-components";
import styles from "./SettingPage.module.css";

/// modpack_id: global or modpack id
function SettingPage({ modpack_id = "global" }: { modpack_id?: string }) {
  const selectJavaId = useId("select-java-version");

  return (
    <div className={styles.main_div}>
      <div className={styles.flex}>
        <Label htmlFor={selectJavaId} style={{ textAlignLast: "left" }}>
          Java 版本：
        </Label>
        <Select id={selectJavaId}>
          <option>8.0</option>
          <option>22</option>
        </Select>
      </div>
    </div>
  );
}

export default SettingPage;
