import {
  Button,
  Menu,
  MenuTrigger,
  MenuPopover,
  MenuList,
  MenuItem,
} from "@fluentui/react-components";
import styles from "./Games.module.css";
import {
  EditPersonRegular,
  PlayRegular,
  SettingsRegular,
} from "@fluentui/react-icons";
import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { style } from "motion/react-client";

function Games() {
  const [expanded, setExpanded] = useState(false);

  return (
    <div>
      <div className={styles.menu}>
        <Menu open={expanded}>
          <MenuTrigger>
            <Button
              onMouseEnter={() => setExpanded(true)}
              onMouseLeave={() => setExpanded(false)}
              icon={<PlayRegular />}
              iconPosition="after"
              size="large"
              style={{
                height: "32px",
              }}
            >
              <motion.div
                animate={{ width: expanded ? 60 : 0 }} // 宽度动画
                style={{
                  display: "flex",
                  alignItems: "center",
                  overflow: "hidden",
                  right: 0,
                  transformOrigin: "right",
                }}
              >
                {expanded && (
                  <motion.span
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    style={{ marginLeft: 8 }}
                  >
                    Play
                  </motion.span>
                )}
              </motion.div>
            </Button>
          </MenuTrigger>
          <MenuPopover
            onMouseEnter={() => setExpanded(true)}
            onMouseLeave={() => setExpanded(false)}
            style={{ overflow: "hidden", alignItems: "flex-end" }}
          >
            <AnimatePresence>
              {expanded && (
                <motion.div
                  initial={{ opacity: 0, y: 10, height: 0 }}
                  animate={{ opacity: 1, y: 0, height: "auto" }}
                  exit={{ opacity: 0, y: -10, height: 0 }}
                >
                  <MenuList
                    style={{
                      display: "flex",
                      flexDirection: "column",
                      gap: "10px",
                      alignItems: "flex-end",
                    }}
                  >
                    <MenuItem
                      className={styles.menu_item}
                      icon={<SettingsRegular />}
                    >
                      setting
                    </MenuItem>
                    <MenuItem
                      className={styles.menu_item}
                      icon={<EditPersonRegular />}
                    >
                      edit
                    </MenuItem>
                  </MenuList>
                </motion.div>
              )}
            </AnimatePresence>
          </MenuPopover>
        </Menu>
      </div>
    </div>
  );
}

export default Games;
