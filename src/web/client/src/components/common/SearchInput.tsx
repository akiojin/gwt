import { Search } from "lucide-react";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

interface SearchInputProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  className?: string;
  count?: { filtered: number; total: number };
}

export function SearchInput({
  value,
  onChange,
  placeholder = "Search...",
  className,
  count,
}: SearchInputProps) {
  return (
    <div className={cn("flex items-center gap-3", className)}>
      <div className="relative flex-1">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          className="pl-9"
        />
      </div>
      {count && (
        <span className="text-sm text-muted-foreground">
          {count.filtered} / {count.total}
        </span>
      )}
    </div>
  );
}
