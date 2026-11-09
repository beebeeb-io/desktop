// The office canvas's document and slide shapes (B08): what Canvas.tsx draws. The board's fixtures
// (../../../source/fixtures/office.ts) fill them with the board's example documents.

export interface DocSection { heading: string; text: string; selected?: string }
export interface DocFixture { title: string; sub: string; outline: [string, number][]; sections: DocSection[] }
export interface SlideFixture { title: string; sub?: string; points: string[]; photo: boolean }
