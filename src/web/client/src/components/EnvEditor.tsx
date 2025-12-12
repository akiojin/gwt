import React from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";

export interface EnvRow {
  id: string;
  key: string;
  value: string;
  importedFromOs?: boolean;
  lastUpdated?: string | null;
}

interface EnvEditorProps {
  title: string;
  rows: EnvRow[];
  onChange: (rows: EnvRow[]) => void;
  description?: string;
  allowAdd?: boolean;
  emptyLabel?: string;
}

const KEY_PATTERN = /^[A-Z0-9_]+$/;

export function createEnvRow(variable?: Partial<EnvRow>): EnvRow {
  const row: EnvRow = {
    id:
      variable?.id ??
      `env-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    key: variable?.key ?? "",
    value: variable?.value ?? "",
  };

  if (typeof variable?.importedFromOs === "boolean") {
    row.importedFromOs = variable.importedFromOs;
  }
  if (variable?.lastUpdated) {
    row.lastUpdated = variable.lastUpdated;
  }

  return row;
}

function isInvalidKey(row: EnvRow): boolean {
  if (!row.key) return true;
  return !KEY_PATTERN.test(row.key);
}

export function EnvEditor({
  title,
  rows,
  onChange,
  description,
  allowAdd = true,
  emptyLabel = "環境変数はまだありません",
}: EnvEditorProps) {
  const handleFieldChange = (
    id: string,
    field: "key" | "value",
    value: string,
  ) => {
    onChange(
      rows.map((row) =>
        row.id === id
          ? {
              ...row,
              [field]:
                field === "key"
                  ? value.toUpperCase().replace(/[^A-Z0-9_]/g, "_")
                  : value,
            }
          : row,
      ),
    );
  };

  const handleRemove = (id: string) => {
    onChange(rows.filter((row) => row.id !== id));
  };

  const handleAdd = () => {
    onChange([...rows, createEnvRow()]);
  };

  return (
    <div className="space-y-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="font-semibold">{title}</h3>
          {description && (
            <p className="mt-1 text-sm text-muted-foreground">{description}</p>
          )}
        </div>
        {allowAdd && (
          <Button variant="secondary" size="sm" onClick={handleAdd}>
            変数を追加
          </Button>
        )}
      </div>

      {rows.length === 0 ? (
        <p className="py-4 text-center text-sm text-muted-foreground">{emptyLabel}</p>
      ) : (
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>キー</TableHead>
                <TableHead>値</TableHead>
                <TableHead className="w-24 text-right">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row) => {
                const keyInvalid = isInvalidKey(row);
                return (
                  <TableRow
                    key={row.id}
                    className={cn(keyInvalid && "bg-destructive/10")}
                  >
                    <TableCell className="space-y-1">
                      <Input
                        type="text"
                        value={row.key}
                        onChange={(event) =>
                          handleFieldChange(row.id, "key", event.target.value)
                        }
                        placeholder="EXAMPLE_KEY"
                        className={cn(keyInvalid && "border-destructive")}
                      />
                      <div className="flex flex-wrap items-center gap-2">
                        {row.importedFromOs && (
                          <Badge variant="outline" className="text-xs">
                            OSから取り込み
                          </Badge>
                        )}
                        {row.lastUpdated && (
                          <span className="text-xs text-muted-foreground">
                            更新: {new Date(row.lastUpdated).toLocaleString()}
                          </span>
                        )}
                      </div>
                      {keyInvalid && (
                        <p className="text-xs text-destructive">
                          A-Z,0-9,_ のみ使用できます
                        </p>
                      )}
                    </TableCell>
                    <TableCell>
                      <Input
                        type="text"
                        value={row.value}
                        onChange={(event) =>
                          handleFieldChange(row.id, "value", event.target.value)
                        }
                        placeholder="値"
                      />
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleRemove(row.id)}
                      >
                        削除
                      </Button>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}
